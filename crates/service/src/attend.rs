//! The three verbs an embedding app needs from the service, and the reading that tells it whether
//! any of it took effect (6j6v.5zst).
//!
//! Register, deregister, read. Deliberately no more: the service decides for itself WHAT to do with
//! a workspace it attends (sync it, keep its deadlines) — the app only says which workspaces exist
//! and asks what happened.

use std::path::{Path, PathBuf};

use nxs_foundation::error::Result;

use crate::heartbeat::{self, ServiceState, WorkspaceHealth};
use crate::home::ServiceHome;
use crate::program::ProgramState;
use crate::registry::{self, WorkspaceEntry};

/// The registry key a heartbeat entry belongs to (nxf 6j6v.s4re). The service's sweep labels each
/// workspace with the `.nxs` DIRECTORY it opened, while the registry — and every reader here —
/// names the root that holds it; an entry written either way is read as its root.
fn health_key(path: &str) -> String {
    let key = registry::key_for(path);
    match Path::new(&key).file_name() {
        Some(name) if name == ".nxs" => Path::new(&key)
            .parent()
            .map(|root| registry::key_for(&root.to_string_lossy()))
            .unwrap_or(key),
        _ => key,
    }
}

/// What the service is doing for one workspace, from the outside.
///
/// `registered` and `service` answer different questions and both matter: a workspace can be
/// registered while the service is down (nothing will happen until it starts), and the service can
/// be up while a workspace is unregistered (it does not know the workspace exists). An app that
/// reports "syncing" on the strength of a successful `register` alone would be lying in both cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attendance {
    /// The workspace is in the registry the service sweeps.
    pub registered: bool,
    /// Whether the service process itself is up — including the honest `Unknown` on a platform
    /// with no liveness probe.
    pub service: ServiceState,
    /// When the running service started, from its heartbeat. `None` when there is no heartbeat.
    pub started_at: Option<String>,
    /// When the service last completed a pass over ANY workspace. `None` when there is no
    /// heartbeat. Distinct from `workspace.last_ok` below: a service that is alive and sweeping but
    /// has never reached THIS workspace has a `last_pass_at` and no `workspace`.
    pub last_pass_at: Option<String>,
    /// What the service last did with THIS workspace, if it has reached it at all.
    pub workspace: Option<WorkspaceHealth>,
}

impl ServiceHome {
    /// Ask the service to attend `root` (the directory holding `.nxs/`).
    ///
    /// Idempotent — returns whether the registry actually changed, so a caller that registers on
    /// every project open does not rewrite the file every time. The path is absolutized against
    /// `cwd` and normalized, but NOT existence-checked and NOT canonicalized: registration is lazy
    /// by design (the same contract the per-call `workspace` override has), and resolving symlinks
    /// would rewrite the very string the caller hands back later.
    pub fn register(&self, root: &Path, cwd: &Path) -> Result<bool> {
        let path = registry::absolutize(root, cwd);
        let entry = WorkspaceEntry {
            name: registry::name_for(&path),
            path,
        };
        registry::upsert_into(&self.registry(), &entry)
    }

    /// Stop the service attending `root`. Returns whether an entry was actually removed;
    /// deregistering something that was never registered is a successful no-op.
    ///
    /// This is the half that did not exist anywhere before, and its absence is measurable: the
    /// registry on one developer machine had grown to 98 entries for workspaces that no longer
    /// existed, because every path into it added and none removed. An app that opens projects all
    /// day rebuilds that in an afternoon.
    ///
    /// It removes the workspace from the service's list ONLY. Nothing inside the workspace is
    /// touched — its `.nxs/sync.toml` stream binding survives, so registering again resumes exactly
    /// where it left off rather than starting a fresh sync.
    pub fn deregister(&self, root: &Path, cwd: &Path) -> Result<bool> {
        registry::remove_from(&self.registry(), &registry::absolutize(root, cwd))
    }

    /// Is the service up, does it know about `root`, and what did it last do with it?
    pub fn attendance(&self, root: &Path, cwd: &Path) -> Result<Attendance> {
        let key = registry::absolutize(root, cwd);
        let registered = registry::load_from(&self.registry())?
            .iter()
            .any(|e| registry::key_for(&e.path) == key);
        let hb = heartbeat::read_from(&self.heartbeat())?;
        Ok(Attendance {
            registered,
            service: heartbeat::state(hb.as_ref()),
            started_at: hb.as_ref().map(|h| h.started_at.clone()),
            last_pass_at: hb.as_ref().map(|h| h.last_pass_at.clone()),
            workspace: hb.and_then(|h| {
                h.workspaces
                    .into_iter()
                    .find(|w| health_key(&w.path) == key)
            }),
        })
    }

    /// Every workspace the service is currently set to attend.
    pub fn attended(&self) -> Result<Vec<WorkspaceEntry>> {
        registry::load_from(&self.registry())
    }

    /// **Is anything actually doing this workspace's work?** (nxf 6j6v.0j12)
    ///
    /// [`attendance`](ServiceHome::attendance) reports the two halves separately, which is right
    /// for something that renders them. This is the one-line verdict a caller BRANCHES on, and it
    /// exists because until now the answer was nowhere: `nxs sync daemon status` said it on
    /// request, and nothing asked.
    ///
    /// `None` means nothing to report. It is `None` for an UNREGISTERED workspace on purpose:
    /// nobody ever asked the service to attend it, so a service that is not attending it is not a
    /// fault — it is the arrangement. A workspace that HAS been registered is a standing request,
    /// and a standing request nobody is serving is exactly what has been silent.
    ///
    /// The order of the two answers is the diagnosis rather than a priority: the fault is always
    /// "nothing is running", and a dangling program alias is the CAUSE when there is one, which is
    /// the difference between a message somebody can act on and one that only restates the symptom.
    /// A live service whose alias has since gone dead is deliberately not a fault here — it is
    /// keeping time right now — and it is not lost either: it is what every `nxs` invocation says
    /// (nxf 6j6v.dcpk (c)), because it is a machine-wide fact and not this workspace's.
    pub fn fault(&self, root: &Path, cwd: &Path) -> Result<Option<ServiceFault>> {
        let attendance = self.attendance(root, cwd)?;
        if !attendance.registered {
            return Ok(None);
        }
        // `Unknown` is a platform that cannot probe its own service, not a service that is down —
        // the same reading `crates/chat`'s attendance check makes. Reporting a fault there would
        // put a warning on every call on that platform, forever, for a question nobody can answer.
        //
        // `Unconfirmed` is NOT in that company (nxf 6j6v.kcan), and the difference is what the
        // reader can do about it. `Unknown` is the whole platform's permanent condition — nothing
        // will ever be learned, on any machine, so a warning is noise forever. `Unconfirmed` names
        // one machine's live situation, and since nxf 6j6v.d43g it names exactly one: the service
        // is gone and its id has been handed on, so this machine has no clock. So it is said — in
        // its own words, never in `NotRunning`'s, because the pid a reader can see is not theirs.
        match attendance.service {
            ServiceState::Running | ServiceState::Unknown => Ok(None),
            ServiceState::Unconfirmed => Ok(Some(ServiceFault::Unconfirmed)),
            ServiceState::NotRunning => Ok(Some(self.not_running_fault())),
        }
    }

    /// The fault a service that is NOT running IS, diagnosed against the program alias.
    ///
    /// Split out so the two callers that already know the service is down — [`fault`](Self::fault)
    /// and `crates/chat`'s attendance check — reach the same diagnosis and the same sentence
    /// without either re-reading the heartbeat the other has just read.
    pub fn not_running_fault(&self) -> ServiceFault {
        match self.program_state() {
            ProgramState::Dangling(target) => ServiceFault::ProgramMissing {
                link: self.program(),
                target,
            },
            _ => ServiceFault::NotRunning,
        }
    }
}

/// One workspace attended by more than one instance, and by whom.
///
/// See [`ServiceHome::attended_by_more_than_one`] for what it means and why it is reported rather
/// than prevented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedWorkspace {
    /// The workspace path, as this instance's registry spells it.
    pub path: String,
    /// Every instance attending it, including this one, sorted — so the same overlap reads the same
    /// whichever instance reports it.
    pub instances: Vec<String>,
}

impl ServiceHome {
    /// **Which of this instance's workspaces somebody else is also attending** (nxf 6j6v.gd9p).
    ///
    /// Separate homes make two instances structurally unable to share a lock, a registry or a
    /// heartbeat. They do NOT stop a person registering one workspace with both — the registries
    /// are two files and neither service is in a position to police the other's. What follows if
    /// they do is not a stalemate but a DUPLICATE: both services sweep that workspace and both read
    /// its deadline book, so a window that comes due can be taken by whichever gets there first and,
    /// in the moment two ticks overlap, started twice.
    ///
    /// So the answer to "what holds when a workspace is registered with two instances" is neither
    /// prevention nor silence: it is reported, everywhere the registry is read — `nxs sync daemon
    /// status`, the bind that creates the overlap, and the running service's own log. Registering
    /// with exactly one instance is the arrangement; this is what says when it is not the case.
    ///
    /// **Compared by registry KEY, which is a string** — the same normalisation every other reader
    /// of this file uses (`attendance`, `deregister`, the daemon's own sweep), and it inherits that
    /// contract's one limit: two DIFFERENT spellings of one directory — a symlinked path, `/var`
    /// versus `/private/var` — read as two workspaces, and no overlap is reported. The registry
    /// deliberately stores what it was handed rather than canonicalising (its own doc: the caller
    /// gets its own string back), and an overlap check that resolved symlinks while nothing beside
    /// it did would disagree with every other reader. In practice both registrations come from the
    /// same `absolutize(root, cwd)` and agree; a hand-edited registry is the case that does not,
    /// and it is exactly the case that file is invited to be.
    ///
    /// **A SISTER's unreadable registry contributes nothing; THIS instance's is an `Err`** (review
    /// of PR #395, Integrity #2). The two are not the same fact and used to be swallowed together.
    /// A sister we cannot read is a diagnostic about somebody else, and a diagnostic beside an
    /// answer that has already been given must never be able to fail one. Our OWN registry being
    /// unreadable means this check did not run at all — and reporting that as "no overlap" is the
    /// silent answer the whole overlap machinery exists to avoid. Each caller decides what to do
    /// with it; none of them may drop it without saying why.
    pub fn attended_by_more_than_one(&self) -> Result<Vec<SharedWorkspace>> {
        let mine = registry::load_from(&self.registry())?;
        let siblings: Vec<(String, Vec<WorkspaceEntry>)> = self
            .siblings()
            .into_iter()
            .filter_map(|h| {
                let entries = registry::load_from(&h.registry()).ok()?;
                Some((h.instance().name(), entries))
            })
            .collect();

        let mut shared: Vec<SharedWorkspace> = Vec::new();
        for entry in &mine {
            let key = registry::key_for(&entry.path);
            let mut instances: Vec<String> = siblings
                .iter()
                .filter(|(_, entries)| entries.iter().any(|e| registry::key_for(&e.path) == key))
                .map(|(name, _)| name.clone())
                .collect();
            if instances.is_empty() {
                continue;
            }
            instances.push(self.instance().name());
            instances.sort();
            shared.push(SharedWorkspace {
                path: entry.path.clone(),
                instances,
            });
        }
        shared.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(shared)
    }
}

/// The ONE sentence about a doubly-attended workspace, so `status`, `bind` and the service's own
/// log cannot describe the same machine differently — the same rule
/// [`ServiceFault::detail`] follows.
///
/// `None` when there is nothing to say, which is the ordinary case.
pub fn shared_workspace_note(shared: &[SharedWorkspace]) -> Option<String> {
    let first = shared.first()?;
    let more = match shared.len() {
        1 => String::new(),
        n => format!(" (and {} more)", n - 1),
    };
    Some(format!(
        "{} is registered with more than one nexus-flow service instance ({}){more}. Both attend \
         it, so both read its deadline book and a window that comes due can be started twice. \
         Register a workspace with exactly one instance: `nxs sync unregister` it from the others.",
        first.path,
        first.instances.join(", ")
    ))
}

/// Why the background service attending a workspace is not doing its work (nxf 6j6v.0j12).
///
/// **It carries its own sentence** ([`detail`](ServiceFault::detail)) rather than leaving each
/// caller to write one, because the sentence is the deliverable: the fault has to say what it
/// COSTS, and what it costs is both halves of the service's job — which is the half of this that
/// every earlier message got wrong by naming only the clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceFault {
    /// Nothing is running, and the reason is visible: the program alias names a file that is not
    /// there, so launchd cannot start anything (nxf 6j6v.dcpk).
    ProgramMissing {
        /// The alias — `~/.nexusflow/bin/nexus-flow`.
        link: PathBuf,
        /// What it points at, which does not exist.
        target: PathBuf,
    },
    /// Nothing is running, and the alias is not the reason: never installed, booted out, or in a
    /// crash loop.
    NotRunning,
    /// **Nobody could confirm either way** (nxf 6j6v.kcan): a process holds the pid the heartbeat
    /// records, and it cannot be shown to be the service that wrote it. See
    /// [`ServiceState::Unconfirmed`] for what produces it — and, since nxf 6j6v.d43g, for the
    /// second reading it USED to have and no longer does.
    ///
    /// It is a fault rather than a silence because it is a machine with no clock: the service that
    /// wrote the record is gone. It is not [`NotRunning`](Self::NotRunning) because a live process
    /// holds the id, and a reader told "the process is gone" reaches for a pid that is not theirs.
    Unconfirmed,
}

/// What a service that is not running costs — the SAME two halves in every message, so no caller
/// can name one and forget the other.
///
/// The owner decision of 2026-08-25 made this one process the only clock on the machine; it was
/// already the only sync. Before that decision the clock was self-carrying (`launchd` fired a job
/// per deadline, running service or not) and this failure case did not exist at all.
const BOTH_HALVES: &str = "declared windows will not fire — a round that should release hangs \
                           indefinitely — and this machine's op log neither pushes nor pulls";

impl ServiceFault {
    /// The whole sentence: what is wrong, what it costs, and the way out.
    pub fn detail(&self) -> String {
        match self {
            ServiceFault::ProgramMissing { link, target } => format!(
                "the nexus-flow background service cannot start: its program link {} points at {}, \
                 which is not there. Until it does, {BOTH_HALVES}. Re-point it by running \
                 `nxs sync daemon install` from the binary you want the service to run.",
                link.display(),
                target.display()
            ),
            ServiceFault::NotRunning => format!(
                "no nexus-flow background service is running, and this workspace is registered for \
                 one. Until one starts, {BOTH_HALVES}. Start it with `nxs sync daemon install` \
                 (macOS) or run `nxs sync daemon` under your own supervisor."
            ),
            ServiceFault::Unconfirmed => format!(
                "the nexus-flow background service cannot be confirmed as running: a process holds \
                 the id it recorded, but that process began after the record was made — the \
                 service died and its id was handed on, so what you can see is somebody else's. \
                 {BOTH_HALVES}. Read `nxs sync daemon status` before reaching for that id: start \
                 the service, do not kill the pid."
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heartbeat::{rfc3339, Heartbeat};
    use std::path::PathBuf;
    use std::time::SystemTime;
    use tempfile::TempDir;

    fn home(dir: &TempDir) -> ServiceHome {
        ServiceHome::at(dir.path().join(".nexusflow"))
    }

    fn cwd() -> PathBuf {
        PathBuf::from("/somewhere/else")
    }

    /// Write a heartbeat claiming `pid`, started now, so `attendance` has something to probe.
    fn heartbeat_for(home: &ServiceHome, pid: u32) {
        std::fs::create_dir_all(home.root()).unwrap();
        heartbeat::write_to(
            &home.heartbeat(),
            &Heartbeat {
                pid,
                started_at: rfc3339(SystemTime::now()),
                last_pass_at: rfc3339(SystemTime::now()),
                workspaces: Vec::new(),
                program: None,
                version: None,
                instance: None,
                write_failures: None,
            },
        )
        .unwrap();
    }

    /// A pid that names no process, so the liveness probe answers `Dead` — the state a machine is
    /// in when the service died and left its last heartbeat behind.
    const DEAD_PID: u32 = u32::MAX;

    #[test]
    fn an_unregistered_workspace_reports_no_fault_however_dead_the_service_is() {
        // Not politeness — the arrangement. Nobody asked the service to attend this workspace, so
        // a service that is not attending it is doing exactly what was asked of it.
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        heartbeat_for(&home, DEAD_PID);
        assert_eq!(
            home.fault(Path::new("/proj/alpha"), &cwd()).unwrap(),
            None,
            "an unregistered workspace is not a standing request"
        );
    }

    #[test]
    fn a_registered_workspace_with_nothing_running_is_a_fault_that_names_both_halves() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        home.register(Path::new("/proj/alpha"), &cwd()).unwrap();
        heartbeat_for(&home, DEAD_PID);
        let fault = home
            .fault(Path::new("/proj/alpha"), &cwd())
            .unwrap()
            .expect("a registered workspace with a dead service is a fault");
        assert_eq!(fault, ServiceFault::NotRunning);
        let detail = fault.detail();
        assert!(
            detail.contains("windows will not fire"),
            "the clock half: {detail}"
        );
        assert!(
            detail.contains("neither pushes nor pulls"),
            "the sync half — the one every earlier message left out: {detail}"
        );
        assert!(
            detail.contains("nxs sync daemon"),
            "and the way out: {detail}"
        );
    }

    /// A heartbeat naming a LIVE process that did not write it — the `Unconfirmed` shape: this
    /// test process's own pid, with a start instant far outside the identity slack.
    fn unconfirmed_heartbeat(home: &ServiceHome) {
        std::fs::create_dir_all(home.root()).unwrap();
        heartbeat::write_to(
            &home.heartbeat(),
            &Heartbeat {
                pid: std::process::id(),
                started_at: "2020-01-01T00:00:00Z".into(),
                last_pass_at: "2020-01-01T00:00:00Z".into(),
                workspaces: Vec::new(),
                program: None,
                version: None,
                instance: None,
                write_failures: None,
            },
        )
        .unwrap();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux"))]
    fn an_unconfirmable_service_is_still_a_fault_but_never_called_a_dead_one() {
        // nxf 6j6v.kcan. The standing warning stays — a pid handed on after a long-dead service
        // (6j6v.0wvp) must not go quiet — but the sentence may no longer assert the process is
        // gone, because the other thing that produces this state is a service that is working.
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        home.register(Path::new("/proj/alpha"), &cwd()).unwrap();
        unconfirmed_heartbeat(&home);

        let fault = home
            .fault(Path::new("/proj/alpha"), &cwd())
            .unwrap()
            .expect("a state nobody can confirm is still worth saying out loud");
        assert_eq!(fault, ServiceFault::Unconfirmed);
        let detail = fault.detail();
        assert!(
            !detail.contains("no nexus-flow background service is running"),
            "that is the claim this state exists to stop making: {detail}"
        );
        assert!(
            detail.contains("nxs sync daemon status"),
            "and it points at the verb that shows the whole reading: {detail}"
        );
        // nxf 6j6v.d43g: nor may it still OFFER the reading it no longer has. A service that
        // stamped itself late reads as running now, so naming that possibility here sends a reader
        // looking for a hang that cannot be there — and softens the one thing this fault says.
        assert!(
            !detail.contains("stamped itself late"),
            "the reading this state lost: {detail}"
        );
        assert!(
            detail.contains("handed on"),
            "and the one it kept, named: {detail}"
        );
    }

    #[test]
    fn a_live_service_is_no_fault_at_all() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        home.register(Path::new("/proj/alpha"), &cwd()).unwrap();
        // This process is alive by construction, and it wrote this heartbeat, so the identity
        // check (start time within the slack) passes too.
        heartbeat_for(&home, std::process::id());
        assert_eq!(home.fault(Path::new("/proj/alpha"), &cwd()).unwrap(), None);
    }

    #[test]
    #[cfg(unix)]
    fn a_dead_alias_is_reported_as_the_cause_rather_than_as_a_bare_not_running() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        home.register(Path::new("/proj/alpha"), &cwd()).unwrap();
        heartbeat_for(&home, DEAD_PID);
        let gone = tmp.path().join("target").join("debug").join("nxs");
        std::fs::create_dir_all(home.bin()).unwrap();
        std::os::unix::fs::symlink(&gone, home.program()).unwrap();

        let fault = home
            .fault(Path::new("/proj/alpha"), &cwd())
            .unwrap()
            .unwrap();
        assert_eq!(
            fault,
            ServiceFault::ProgramMissing {
                link: home.program(),
                target: gone.clone(),
            },
            "the diagnosis, not the symptom"
        );
        let detail = fault.detail();
        assert!(detail.contains(&gone.display().to_string()), "{detail}");
        assert!(
            detail.contains("windows will not fire") && detail.contains("neither pushes nor pulls"),
            "both halves here too: {detail}"
        );
    }

    #[test]
    fn registering_a_workspace_puts_it_on_the_list_the_service_sweeps() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        assert!(home
            .register(Path::new("/proj/alpha"), &cwd())
            .expect("register succeeds"));
        assert_eq!(
            home.attended().unwrap(),
            vec![WorkspaceEntry {
                name: "alpha".into(),
                path: "/proj/alpha".into()
            }]
        );
    }

    #[test]
    fn registering_the_same_workspace_twice_changes_nothing_the_second_time() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        assert!(home.register(Path::new("/proj/alpha"), &cwd()).unwrap());
        assert!(
            !home.register(Path::new("/proj/alpha"), &cwd()).unwrap(),
            "an app that registers on every project open must not rewrite the registry each time"
        );
    }

    #[test]
    fn deregistering_takes_it_off_again_and_leaves_the_others() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        home.register(Path::new("/proj/alpha"), &cwd()).unwrap();
        home.register(Path::new("/proj/beta"), &cwd()).unwrap();
        assert!(home.deregister(Path::new("/proj/alpha"), &cwd()).unwrap());
        assert_eq!(
            home.attended()
                .unwrap()
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/proj/beta"]
        );
    }

    #[test]
    fn deregistering_something_that_was_never_registered_is_not_an_error() {
        let tmp = TempDir::new().unwrap();
        assert!(!home(&tmp)
            .deregister(Path::new("/proj/ghost"), &cwd())
            .expect("a no-op deregister still succeeds"));
    }

    #[test]
    fn a_relative_path_is_resolved_against_the_working_directory_it_was_given() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        home.register(Path::new("alpha"), Path::new("/work"))
            .unwrap();
        assert_eq!(home.attended().unwrap()[0].path, "/work/alpha");
        assert!(
            home.deregister(Path::new("/work/alpha"), &cwd()).unwrap(),
            "the absolute form names the same workspace the relative one registered"
        );
    }

    #[test]
    fn an_unregistered_workspace_with_no_service_reads_as_neither() {
        let tmp = TempDir::new().unwrap();
        let got = home(&tmp)
            .attendance(Path::new("/proj/alpha"), &cwd())
            .unwrap();
        assert_eq!(
            got,
            Attendance {
                registered: false,
                service: ServiceState::NotRunning,
                started_at: None,
                last_pass_at: None,
                workspace: None,
            }
        );
    }

    #[test]
    fn a_registered_workspace_with_no_service_running_says_exactly_that() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        home.register(Path::new("/proj/alpha"), &cwd()).unwrap();
        let got = home.attendance(Path::new("/proj/alpha"), &cwd()).unwrap();
        assert!(got.registered, "the registry entry is there");
        assert_eq!(
            got.service,
            ServiceState::NotRunning,
            "but nothing is running to act on it — an app must be able to tell these apart"
        );
        assert_eq!(got.workspace, None);
    }

    #[test]
    fn the_reading_carries_what_the_service_last_did_with_this_workspace() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        home.register(Path::new("/proj/alpha"), &cwd()).unwrap();
        let health = WorkspaceHealth {
            path: "/proj/alpha".into(),
            last_ok: Some("2026-08-25T10:05:00Z".into()),
            last_error: None,
            pushed: 2,
            pulled: 7,
        };
        heartbeat::write_to(
            &home.heartbeat(),
            &Heartbeat {
                // This very process, AND a start instant that matches its real one — since
                // 6j6v.0wvp the probe checks both, so a fabricated `started_at` would (correctly)
                // read as somebody else's process.
                pid: std::process::id(),
                started_at: rfc3339(SystemTime::now()),
                last_pass_at: "2026-08-25T10:05:00Z".into(),
                workspaces: vec![
                    health.clone(),
                    WorkspaceHealth {
                        path: "/proj/beta".into(),
                        last_ok: None,
                        last_error: Some("relay refused".into()),
                        pushed: 0,
                        pulled: 0,
                    },
                ],
                program: None,
                version: None,
                instance: None,
                write_failures: None,
            },
        )
        .unwrap();

        let got = home.attendance(Path::new("/proj/alpha"), &cwd()).unwrap();
        assert!(got.registered);
        assert!(
            got.started_at.is_some(),
            "the reading carries when the service started"
        );
        assert_eq!(got.last_pass_at.as_deref(), Some("2026-08-25T10:05:00Z"));
        assert_eq!(
            got.workspace,
            Some(health),
            "the sibling workspace's health must not be reported as this one's"
        );
        if cfg!(unix) {
            assert_eq!(got.service, ServiceState::Running);
        }
    }

    /// The shape the REAL service writes (nxf 6j6v.s4re): its sweep labels a workspace with the
    /// `.nxs` directory it opened, not the root the registry and this reading speak. The test above
    /// writes the root, a heartbeat no service produces — which is how the reading stayed green
    /// while returning `None` for every real pass.
    #[test]
    fn the_reading_finds_the_workspace_under_the_nxs_directory_the_service_labels_it_with() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        home.register(Path::new("/proj/alpha"), &cwd()).unwrap();
        let health = WorkspaceHealth {
            path: "/proj/alpha/.nxs".into(),
            last_ok: None,
            last_error: Some("sync failed: relay refused".into()),
            pushed: 0,
            pulled: 0,
        };
        heartbeat_with(&home, vec![health.clone()]);
        let got = home.attendance(Path::new("/proj/alpha"), &cwd()).unwrap();
        assert_eq!(got.workspace, Some(health));
        // …and never a sibling whose name merely STARTS with this one.
        let got = home.attendance(Path::new("/proj/alph"), &cwd()).unwrap();
        assert_eq!(got.workspace, None);
    }

    fn heartbeat_with(home: &ServiceHome, workspaces: Vec<WorkspaceHealth>) {
        heartbeat::write_to(
            &home.heartbeat(),
            &Heartbeat {
                pid: std::process::id(),
                started_at: rfc3339(SystemTime::now()),
                last_pass_at: "2026-08-25T10:05:00Z".into(),
                workspaces,
                program: None,
                version: None,
                instance: None,
                write_failures: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn a_heartbeat_from_a_process_that_is_gone_does_not_read_as_running() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        heartbeat::write_to(
            &home.heartbeat(),
            &Heartbeat {
                pid: u32::MAX,
                started_at: "2026-08-25T10:00:00Z".into(),
                last_pass_at: "2026-08-25T10:05:00Z".into(),
                workspaces: Vec::new(),
                program: None,
                version: None,
                instance: None,
                write_failures: None,
            },
        )
        .unwrap();
        let got = home.attendance(Path::new("/proj/alpha"), &cwd()).unwrap();
        let expected = if cfg!(unix) {
            ServiceState::NotRunning
        } else {
            ServiceState::Unknown
        };
        assert_eq!(got.service, expected);
        assert_eq!(
            got.last_pass_at.as_deref(),
            Some("2026-08-25T10:05:00Z"),
            "the rest of a stale heartbeat is still useful diagnostics"
        );
    }
}

#[cfg(test)]
mod shared_tests {
    use super::*;
    use crate::instance::Instance;
    use tempfile::TempDir;

    /// A `$HOME` holding two instances' homes, each with its own registry.
    fn homes(tmp: &TempDir) -> (ServiceHome, ServiceHome) {
        let dev = Instance::named("nexus-flow-dev").unwrap();
        let prod = ServiceHome::at_instance(
            tmp.path().join(Instance::production().home_dir()),
            Instance::production(),
        );
        let dev = ServiceHome::at_instance(tmp.path().join(dev.home_dir()), dev);
        for h in [&prod, &dev] {
            std::fs::create_dir_all(h.root()).unwrap();
        }
        (prod, dev)
    }

    #[test]
    fn a_workspace_registered_with_one_instance_is_reported_by_nobody() {
        let tmp = TempDir::new().unwrap();
        let (prod, dev) = homes(&tmp);
        prod.register(Path::new("/w/alpha"), Path::new("/"))
            .unwrap();
        dev.register(Path::new("/w/beta"), Path::new("/")).unwrap();
        assert!(prod.attended_by_more_than_one().unwrap().is_empty());
        assert!(dev.attended_by_more_than_one().unwrap().is_empty());
        assert_eq!(shared_workspace_note(&[]), None);
    }

    #[test]
    fn a_workspace_registered_with_two_instances_is_named_by_both_of_them_the_same_way() {
        let tmp = TempDir::new().unwrap();
        let (prod, dev) = homes(&tmp);
        for h in [&prod, &dev] {
            h.register(Path::new("/w/shared"), Path::new("/")).unwrap();
        }
        prod.register(Path::new("/w/mine-only"), Path::new("/"))
            .unwrap();

        let from_prod = prod.attended_by_more_than_one().unwrap();
        let from_dev = dev.attended_by_more_than_one().unwrap();
        assert_eq!(from_prod.len(), 1, "{from_prod:?}");
        assert_eq!(from_prod[0].path, "/w/shared");
        assert_eq!(
            from_prod[0].instances,
            vec!["nexus-flow".to_string(), "nexus-flow-dev".to_string()]
        );
        assert_eq!(
            from_prod, from_dev,
            "the same overlap must read identically from either side, or two services describe \
             one machine differently"
        );

        let note = shared_workspace_note(&from_prod).expect("an overlap is always worth a note");
        assert!(note.contains("/w/shared"), "{note}");
        assert!(note.contains("nexus-flow-dev"), "{note}");
        assert!(
            note.contains("started twice"),
            "the note must say what it COSTS, not only that it is so: {note}"
        );
        // The verb as the CLI actually spells it — a note that names a verb this binary does not
        // have is a false map, and the live run of this branch printed exactly that.
        assert!(note.contains("nxs sync unregister"), "{note}");
    }

    #[test]
    fn a_sister_with_no_registry_at_all_contributes_nothing_rather_than_failing() {
        let tmp = TempDir::new().unwrap();
        let (prod, _dev) = homes(&tmp);
        prod.register(Path::new("/w/alpha"), Path::new("/"))
            .unwrap();
        // The dev home exists (so `siblings` finds it) and has never registered anything.
        assert!(prod.attended_by_more_than_one().unwrap().is_empty());
    }

    /// The asymmetry, and it is the whole of Integrity #2: a SISTER we cannot read is somebody
    /// else's problem and contributes nothing; OUR OWN unreadable registry means the check never
    /// ran, and reporting that as "no overlap" is exactly the silent answer this machinery exists
    /// to prevent.
    #[test]
    fn a_sisters_broken_registry_is_ignored_and_our_own_is_not() {
        let tmp = TempDir::new().unwrap();
        let (prod, dev) = homes(&tmp);
        prod.register(Path::new("/w/alpha"), Path::new("/"))
            .unwrap();

        std::fs::write(dev.registry(), "this is not toml {{{").unwrap();
        assert!(
            prod.attended_by_more_than_one().unwrap().is_empty(),
            "a sister whose registry will not parse must not stop us answering"
        );

        std::fs::write(prod.registry(), "neither is this {{{").unwrap();
        let err = prod
            .attended_by_more_than_one()
            .expect_err("our own unreadable registry is not the same fact as `no overlap`");
        assert_eq!(err.kind.as_str(), "validation");
        assert!(
            err.msg.contains("workspaces.toml"),
            "and it names the file to go and look at: {}",
            err.msg
        );
    }
}
