//! What EVERY init path does about the background service (nxf 6j6v.y12q).
//!
//! `nxs init` has put the workspace it sets up on the service's list since nxf 6j6v.npf9, and the
//! module front doors did not: under `--json` or `--quiet` — the way an agent enters — `nxf init`
//! and `nxm init` run the module's own init and never reach the umbrella at all. Since 6j6v.8see
//! that list is where a workspace's DEADLINES come from, so a workspace an agent set up had no
//! clock, and nothing said so. The owner's decision of 2026-08-25 is what settles it: deadlines run
//! through the background service ALWAYS, so every path that creates a workspace registers it.
//!
//! **Two callers register through this; a third shares its note.** `nxf init` and `nxm init` call
//! [`register_workspace`]. The umbrella's own path keeps `nxs::background_service`, which does more
//! than this — it also OFFERS to install the service, which a module front door never does — and
//! writes the registry through its own `Machine` seam; what it takes from here is
//! [`overlap_note`], so all three describe a doubly-attended workspace in the same sentence
//! (review of PR #421, Code Quality #3, which found the earlier wording overstated).
//!
//! **It never fails an init.** Registration is follow-up to a workspace that is already set up —
//! reporting a failed `init` for a workspace that exists would be the worse lie — so a failure is
//! an honest `false` plus a note for the caller to print, exactly as `background_service::apply`
//! decided in npf9.

use std::path::Path;

use nxs_service::ServiceHome;

/// What [`register`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    /// Whether this workspace is on the list the background service attends, after the call.
    pub registered: bool,
    /// What the caller should print as `note:` lines, in order. Empty when there is nothing to
    /// say — which is the ordinary success.
    ///
    /// A LIST because two different things are worth saying and they are independent: registration
    /// failed, and this workspace is now attended by more than one service instance. The second
    /// can only be said about a registration that SUCCEEDED, and saying it is the npf9/gd9p rule —
    /// the moment an overlap is created is the moment to say so.
    pub notes: Vec<String>,
}

/// Put `root` — the PROJECT root, the directory holding `.nxs/` — on the list this machine's
/// background service attends.
///
/// The same string `nxs init` and `nxs sync bind` register, so a workspace registered by one of
/// them and by this is one entry and not two spellings of the same path.
pub fn register(root: &Path) -> Registration {
    let home = match ServiceHome::resolve() {
        Ok(home) => home,
        Err(e) => return Registration::failed(&e.msg),
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    register_into(&home, root, &cwd)
}

/// Register the workspace whose `.nxs/` directory is `nxs_dir` — the form both front doors call,
/// because the PROJECT root is what goes on the list and every caller was deriving it the same way
/// (review of PR #421, Code Quality #2).
///
/// Deriving it here rather than at each call site is what keeps this seam's promise: the umbrella's
/// own `background_service::apply` treats a `.nxs/` with no parent as "not registered", and two
/// call sites saying `unwrap_or(&ws.dir)` quietly said something else — they would have registered
/// the `.nxs` directory itself. Unreachable in practice (a real workspace always has a parent), and
/// a drift in a one-rule seam is worth removing whether or not it can be reached.
pub fn register_workspace(nxs_dir: &Path) -> Registration {
    match nxs_dir.parent() {
        Some(root) => register(root),
        None => Registration::failed("it has no parent directory to register"),
    }
}

/// [`register`] against an explicit home and working directory — the form a test uses, so nothing
/// it does can reach the developer's real `~/.nexusflow` (the leak that registry's own history
/// records at 98 stale entries).
pub fn register_into(home: &ServiceHome, root: &Path, cwd: &Path) -> Registration {
    match home.register(root, cwd) {
        // `register` reports whether the registry CHANGED; a workspace that was already on the
        // list is registered just as much as one this call added, and `init` is re-runnable.
        Ok(_) => Registration {
            registered: true,
            notes: overlap_note(home)
                .into_iter()
                .chain(nxs_service::ignored_instance_env())
                .collect(),
        },
        Err(e) => Registration::failed(&e.msg),
    }
}

/// **Is this workspace now attended by more than one service instance?** — the note every path
/// that WRITES the registry owes (nxf 6j6v.gd9p), spelled once here so the three of them cannot
/// drift apart.
///
/// It is not the only such note any more. [`register_into`] carries a second one beside it since
/// nxf 6j6v.cvpy: WHICH service this workspace landed in, when the environment named another and
/// the running binary gave it no vote (`nxs_service::ignored_instance_env`). Both are sentences
/// about a registry a caller has just changed, and both are `None` in the ordinary case — and the
/// second is `None` in every test, because a test binary is itself a build, which is what keeps
/// this seam's own suite independent of the environment its runner was started in.
///
/// An unreadable own registry yields nothing: the upsert this note follows has just read and
/// rewritten that very file, so a caller that got here cannot have one — and if that ever stops
/// being true, the upsert is the loud one, not the note beside it.
pub fn overlap_note(home: &ServiceHome) -> Option<String> {
    nxs_service::shared_workspace_note(&home.attended_by_more_than_one().ok()?)
}

impl Registration {
    /// The `service:` line of an init summary — the ONE sentence every init path prints about
    /// this, so `nxf init` and `nxm init` cannot describe the same fact differently.
    ///
    /// It names the CONSEQUENCE rather than the mechanism: "registered" alone tells a reader
    /// nothing about why a registry matters, and what it matters for is that since nxf 6j6v.8see
    /// this list is where a workspace's deadlines are kept.
    pub fn summary_line(&self) -> &'static str {
        if self.registered {
            "registered with the background service, which keeps this workspace's deadlines"
        } else {
            "NOT registered — see the note above; this workspace's deadlines will not be kept"
        }
    }

    /// The honest no, with the note that says what it costs.
    fn failed(why: &str) -> Registration {
        Registration {
            registered: false,
            notes: vec![format!(
                "this workspace could not be added to the background service's list ({why}); its \
                 deadlines will not be kept until it is — `nxs init` here adds it"
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn a_workspace_lands_on_the_list_the_service_attends_and_a_re_run_is_a_no_op() {
        // nxf 6j6v.y12q: the whole of the fix, at the seam both front doors call. Before it, a
        // workspace an agent set up with `nxf init --json` was in no registry at all — and since
        // 6j6v.8see the registry is what gives a workspace a clock.
        let tmp = TempDir::new().unwrap();
        let home = ServiceHome::at(tmp.path().join(".nexusflow"));
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();

        let got = register_into(&home, &root, tmp.path());
        assert_eq!(
            got,
            Registration {
                registered: true,
                notes: Vec::new()
            }
        );
        let listed = nxs_service::registry::load_from(&home.registry()).unwrap();
        assert_eq!(listed.len(), 1, "one entry, for this workspace");
        assert!(listed[0].path.ends_with("proj"), "{:?}", listed[0]);

        assert_eq!(
            register_into(&home, &root, tmp.path()),
            Registration {
                registered: true,
                notes: Vec::new()
            },
            "init is re-runnable, so registering twice is a success and not a second entry"
        );
        assert_eq!(
            nxs_service::registry::load_from(&home.registry())
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn registering_into_a_second_instance_says_the_workspace_is_now_attended_twice() {
        // The npf9/gd9p rule, applied to the path this ticket opens: the moment an OVERLAP is
        // created is the moment to say so. Two instances cannot fight over a lock, but both read
        // the same deadline book, so a window that comes due is started twice — and this call is
        // what has just made that true.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let dev = ServiceHome::at_instance(
            tmp.path().join(".nexusflow-dev"),
            nxs_service::Instance::named("nexus-flow-dev").unwrap(),
        );
        assert!(register_into(&dev, &root, tmp.path()).registered);

        let prod = ServiceHome::at(tmp.path().join(".nexusflow"));
        let got = register_into(&prod, &root, tmp.path());
        assert!(got.registered, "the registration itself still succeeds");
        let [note] = &got.notes[..] else {
            panic!(
                "the overlap this call created must be said: {:?}",
                got.notes
            )
        };
        assert!(
            note.contains("more than one nexus-flow service instance"),
            "{note}"
        );
    }

    #[test]
    fn a_workspace_with_no_parent_registers_nothing_rather_than_registering_its_own_nxs_dir() {
        // The drift this entry point exists to remove: `unwrap_or(&ws.dir)` at the call sites would
        // have put the `.nxs` directory itself on the list. `/` is the one path with no parent.
        let got = register_workspace(Path::new("/"));
        assert!(!got.registered);
        assert!(
            got.notes.iter().any(|n| n.contains("no parent")),
            "and it says why: {:?}",
            got.notes
        );
    }

    #[test]
    fn the_summary_line_says_what_the_registration_is_for_either_way() {
        // Both wordings, provable without a registry — and the failing one has to point at the
        // note beside it, because "NOT registered" on its own leaves a reader with nowhere to go.
        let yes = Registration {
            registered: true,
            notes: Vec::new(),
        }
        .summary_line();
        let no = Registration::failed("disk full").summary_line();
        assert!(yes.contains("deadlines"), "{yes}");
        assert!(no.contains("deadlines will not be kept"), "{no}");
        assert!(no.contains("note above"), "{no}");
    }

    #[test]
    fn a_registry_that_cannot_be_written_is_a_note_and_an_honest_no() {
        // Registration is follow-up to a workspace that is already set up: reporting a FAILED
        // `init` for a workspace that exists would be worse than a note — the same rule
        // `background_service::apply` set in nxf 6j6v.npf9. And the note has to say what the
        // consequence is, because "could not register" means nothing to a reader who does not
        // know the registry is where deadlines come from.
        let tmp = TempDir::new().unwrap();
        let home = ServiceHome::at(tmp.path().join(".nexusflow"));
        // A DIRECTORY where the registry file belongs: nothing can be written onto it.
        std::fs::create_dir_all(home.registry()).unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();

        let got = register_into(&home, &root, tmp.path());
        assert!(!got.registered);
        let [note] = &got.notes[..] else {
            panic!("a failure that says nothing is the defect: {:?}", got.notes)
        };
        assert!(
            note.contains("deadlines"),
            "the note must name what is lost, not just what failed: {note}"
        );
    }
}
