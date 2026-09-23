//! What `nxs init` does about the background service (nxf 6j6v.npf9).
//!
//! Until this module the service reached a machine on exactly ONE path: as a side effect of a
//! successful `nxs sync bind` (`sync/mod.rs`, `Autostart`), and the same was true of the registry
//! entry that makes the service look at a workspace at all (`Registrar`). That was right while the
//! service only carried the SYNC — no sync, no sync service. Since 6j6v.8see it also carries the
//! CLOCK, and a workspace with declared windows but no stream has every reason to want it. So both
//! halves belong where a workspace is BORN, not where it is bound to a stream:
//!
//! * **Registration is unconditional.** A workspace that only wants the clock must be on the list
//!   the service sweeps, and being on that list says nothing about syncing anything (the registry
//!   is the DIRECTORY, not the binding — `sync bind` writes `.nxs/sync.toml`, this does not touch
//!   it). An entry without reasons reads as a pinned one under 6j6v.8xpy, which is the compatible
//!   reading, so registering today forecloses nothing that decision might settle.
//! * **Installing the service is offered, never assumed.** On a real terminal it is a question;
//!   under `--json` or in a pipe it is never asked and never done, exactly like the beads
//!   migration's consent gate — the non-interactive way in is an explicit flag.
//!
//! The DECISION is kept apart from the doing: [`plan`] is a pure function of the five facts that
//! settle it, so every branch — including the two that must never prompt — is provable without a
//! terminal, a launchd or a `$HOME`, and [`Machine`] is the seam that keeps the DOING out of a
//! test's way for the same reason `sync::bind` has one.

use std::path::Path;

use nxs_foundation::workspace::{self, Workspace, WorkspaceConfig};

use crate::error::Result;

/// The section of a workspace's `config.toml` the umbrella owns. `nxs` can never collide with a
/// product section: it is the name of the umbrella itself, which is by construction not a module
/// in the roster it fans out over.
const SECTION: &str = "nxs";

/// The key under [`SECTION`] recording what this workspace answered about the background service.
const KEY: &str = "service";

/// The five facts that settle what `init` does about the background service.
///
/// A struct rather than five positional `bool`s: `plan(true, None, false, false, true)` says
/// nothing at a call site, and none of the five is interchangeable with another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offer {
    /// Does this platform have a service installer at all? (launchd — macOS only.)
    pub supported: bool,
    /// The explicit flag: `Some(true)` for `--service`, `Some(false)` for `--no-service`, `None`
    /// when neither was given.
    pub flag: Option<bool>,
    /// Is the service already installed for this instance? Then there is nothing to offer.
    pub installed: bool,
    /// Has THIS workspace already recorded an answer? Then it has been asked once, and asking
    /// again on every re-run is what `init` must not do.
    pub answered: bool,
    /// A real terminal AND not `--json` — the only case in which anything may be asked.
    pub interactive: bool,
}

/// What `init` does about the background service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Install it, and record the answer.
    Install,
    /// Ask the human. Whichever way they answer is recorded.
    Ask,
    /// Record a "no" without asking — the explicit `--no-service`.
    Decline,
    /// Do nothing, and say nothing.
    Skip,
    /// Say by name that this platform has no service to install. Only reached when one was
    /// explicitly ASKED for; an unsupported platform that was not asked says nothing at all.
    Unsupported,
}

/// Decide what to do about the background service. Pure — the whole point of the module.
///
/// The order of the arms IS the contract:
///
/// 1. **A platform with no installer decides nothing.** It cannot install and must not pretend to
///    ask; the only thing it ever says is a named answer to a flag somebody typed.
/// 2. **An explicit flag wins over everything below it**, including an already-installed service
///    (`install` is idempotent and re-points the alias, which is a legitimate thing to ask for)
///    and an already-recorded answer (a flag IS a fresh answer).
/// 3. **An installed service is not offered.** There is nothing to decide, and a "no" to a
///    question about something that is already there would record an answer that is not true.
/// 4. **A recorded answer is not re-asked.** `init` is re-runnable; asking every time is the
///    defect, and a "no" stays a "no" until somebody changes it.
/// 5. **Only a real terminal is asked.** Everything else — `--json`, a pipe, CI, an agent — does
///    nothing and says nothing, which is the DoD's second and third bullets.
pub fn plan(offer: Offer) -> Plan {
    if !offer.supported {
        return match offer.flag {
            Some(true) => Plan::Unsupported,
            _ => Plan::Skip,
        };
    }
    match offer.flag {
        Some(true) => return Plan::Install,
        Some(false) => return Plan::Decline,
        None => {}
    }
    if offer.installed || offer.answered {
        return Plan::Skip;
    }
    if offer.interactive {
        Plan::Ask
    } else {
        Plan::Skip
    }
}

/// What a workspace has RECORDED about the background service, under `[nxs] service`.
///
/// It lives in `config.toml`, which is inside `.nxs/` — a directory that self-ignores (`*`), so
/// this is a MACHINE-LOCAL answer by construction and one developer's "no" never travels to a
/// colleague's checkout through git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The service was installed from this workspace's `init`.
    Installed,
    /// The offer was declined here.
    Declined,
    /// Something else stands under the key — a hand edit (the file invites one), or a newer
    /// binary's vocabulary. It still SETTLES the question here, so `init` does not ask again; it is
    /// reported verbatim rather than guessed at.
    Other(String),
}

impl Answer {
    /// The stored spelling.
    pub fn as_str(&self) -> &str {
        match self {
            Answer::Installed => "installed",
            Answer::Declined => "declined",
            Answer::Other(raw) => raw,
        }
    }

    /// Read one back. Anything that is not one of the two known words is [`Answer::Other`].
    fn from_str(raw: &str) -> Answer {
        match raw {
            "installed" => Answer::Installed,
            "declined" => Answer::Declined,
            other => Answer::Other(other.to_string()),
        }
    }
}

/// What this workspace has recorded, if anything. A non-string value under the key (a hand-edited
/// table, say) reads as no answer: it cannot be reported honestly, and the safe consequence of not
/// understanding it is to ask rather than to assume.
pub fn recorded(config: &WorkspaceConfig) -> Option<Answer> {
    config
        .products
        .get(SECTION)?
        .get(KEY)?
        .as_str()
        .map(Answer::from_str)
}

/// Record `answer` in the workspace's own config, so a re-run does not ask again.
pub fn record(dir: &Path, answer: &Answer) -> Result<bool> {
    workspace::set_product_key(dir, SECTION, KEY, answer.as_str().into())
}

/// The machine-side effects `init` performs about the service, behind a trait.
///
/// The same seam, and the same reason, as `sync::bind`'s `Autostart`/`Registrar`: without it, an
/// in-process test of `init`'s behaviour would install a real launchd agent into the developer's
/// own login session and write into the real `~/.nexusflow/workspaces.toml` — the leak the registry
/// module's own history records at 98 stale entries.
pub trait Machine {
    /// Is a service installed for this instance? `None` on a platform with no installer at all.
    fn installed(&self) -> Option<bool>;
    /// Install it (`nxs sync daemon install`).
    fn install(&self) -> Result<()>;
    /// Put `root` on the list the service attends. Returns whether the registry changed.
    fn register(&self, root: &Path) -> Result<bool>;
    /// Ask the human. Only ever called on [`Plan::Ask`], which only a real terminal reaches.
    fn ask(&self) -> Result<bool>;
}

/// The production [`Machine`]: this machine's launchd agent, this instance's registry, and the
/// shared arrow-key chooser.
pub struct Real;

impl Machine for Real {
    /// `Some` iff there is an installer to answer for. On macOS the plist IS the installation —
    /// `nxs sync daemon install` writes it and `uninstall` removes it — so its presence is the
    /// question, asked of the same instance every other verb resolves.
    fn installed(&self) -> Option<bool> {
        #[cfg(target_os = "macos")]
        {
            nxs_service::launchd::plist_path().ok().map(|p| p.is_file())
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    /// The QUIET install — the one `sync bind` uses — deliberately, not the `nxs sync daemon
    /// install` verb: that one prints its own receipt, and under `nxs init --json` a second object
    /// on stdout would break the one-object contract. What it did is in the summary line instead.
    fn install(&self) -> Result<()> {
        crate::sync::launchd::install_quiet()
    }

    fn register(&self, root: &Path) -> Result<bool> {
        let path = root.to_str().ok_or_else(|| {
            crate::error::NxfError::io(format!(
                "the workspace path is not valid UTF-8: {}",
                root.display()
            ))
        })?;
        let changed = crate::workspaces::register_workspace(path)?;
        // The moment an overlap is CREATED is the moment to say so (nxf 6j6v.gd9p) — the same note
        // `sync bind`'s registrar makes, for the same reason: this `init` may have just made a
        // workspace a second instance's too, and both would then read its deadline book.
        crate::sync::launchd::warn_if_attended_by_more_than_one();
        Ok(changed)
    }

    /// The shared arrow-key chooser, as the ticket requires — the SAME widget as the module
    /// selection and flow's plugin chooser, never a bare `[y/N]` line.
    ///
    /// **The question is not "Sync?".** Since 6j6v.8see this one process carries the clock as well
    /// as the sync, so a question phrased around syncing would have a user decline the clock while
    /// answering about something else (the honesty requirement of R1 S5, 47jy.1745). It names the
    /// deadlines first, and the sync second, because the deadlines are the half that has no other
    /// way to happen.
    fn ask(&self) -> Result<bool> {
        let choices = [
            nxs_ui::Choice::new(
                "yes",
                "one background process keeps the deadlines of every workspace it attends, and \
                 syncs the ones bound to a stream",
            ),
            nxs_ui::Choice::new(
                "no",
                "nothing runs in the background; `nxs sync daemon install` sets it up later",
            ),
        ];
        // **The cursor starts on "yes"**, and that is a decision rather than the default falling
        // out of the call (review of PR #419, Code Quality Info). Two things settle it: since
        // 6j6v.8see this process is the only clock, so a workspace without it silently keeps no
        // deadline at all — the costly answer is the "no", not the "yes"; and the alternative in
        // this codebase is not a safer prompt but `nxs sync bind`, which installs the same service
        // with no question at all. Starting on "yes" is therefore both the useful default and
        // strictly more conservative than the path that exists today. It is still a deliberate
        // keystroke on a labelled option, and `nxs sync daemon uninstall` undoes it.
        match nxs_ui::chooser::choose(
            "Set up the nexus-flow background service on this machine?\n",
            &choices,
            0,
        ) {
            Ok(0) => Ok(true),
            Ok(_) => Ok(false),
            Err(e) => Err(crate::error::NxfError::io(e.to_string())),
        }
    }
}

/// What `init` ended up with, for the summary and for `--json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Whether this workspace is on the list the service attends. `false` only when the registry
    /// could not be written — the attempt itself is unconditional.
    pub registered: bool,
    /// Whether a service is installed on this machine, AFTER this run. `None` on a platform with
    /// no installer, where the question has no answer rather than a negative one.
    pub installed: Option<bool>,
    /// What this workspace has recorded, after this run.
    pub answer: Option<Answer>,
}

/// Do it: register the workspace, then act on [`plan`].
///
/// Never fails. Everything here is follow-up to a workspace that is already set up, exactly like
/// the tail of `sync::bind`: reporting a failed `init` for a workspace that IS initialized would
/// be worse than a note, so a registry write that fails and an install that fails are both a
/// stderr note plus an honest [`Report`], never an `Err`.
pub fn apply(m: &dyn Machine, ws: &Workspace, flag: Option<bool>, interactive: bool) -> Report {
    // The PROJECT root, not `.nxs/` — the same string `sync bind` registers, so a workspace
    // registered by `init` and one registered by `bind` are one entry and not two spellings.
    let registered = match ws.dir.parent() {
        Some(root) => match m.register(root) {
            Ok(_) => true,
            Err(e) => {
                eprintln!(
                    "note: set up, but this workspace could not be added to the background \
                     service's list ({e}); its deadlines will not be kept until it is"
                );
                false
            }
        },
        None => false,
    };

    let before = m.installed();
    let recorded_before = recorded(&ws.config);
    let decided = match plan(Offer {
        // A platform with no installer is exactly a platform that cannot answer "is it
        // installed" — one fact, read once, rather than a second `cfg!` in a second place.
        supported: before.is_some(),
        flag,
        installed: before.unwrap_or(false),
        answered: recorded_before.is_some(),
        interactive,
    }) {
        Plan::Skip => None,
        Plan::Unsupported => {
            eprintln!(
                "note: this platform has no service installer — run `nxs sync daemon` in the \
                 foreground under your own supervisor (systemd, runit, …); it keeps the same \
                 deadlines there"
            );
            None
        }
        Plan::Decline => Some(Answer::Declined),
        Plan::Install => install_now(m),
        Plan::Ask => match m.ask() {
            Ok(true) => install_now(m),
            Ok(false) => Some(Answer::Declined),
            // A cancelled prompt (Esc / Ctrl-C) is not an answer, so nothing is recorded and the
            // next run offers again. Unlike the MODULE chooser, whose cancellation aborts `init`:
            // this question is follow-up to a workspace that is already set up, and refusing to
            // finish it over a keystroke would be the larger surprise.
            Err(e) => {
                eprintln!("note: the background service was not set up ({e})");
                None
            }
        },
    };

    if let Some(answer) = &decided {
        if let Err(e) = record(&ws.dir, answer) {
            eprintln!(
                "note: the background-service answer could not be recorded ({e}); `nxs init` \
                 will ask again next time"
            );
        }
    }

    Report {
        registered,
        // Read AGAIN rather than reusing `before`: an install that just ran changed it, and this
        // line is the one a reader takes as the state of their machine.
        installed: m.installed(),
        answer: decided.or(recorded_before),
    }
}

/// Install the service, turning the outcome into the answer worth recording. A failed install is a
/// note and NO recorded answer: nothing was decided that a re-run should be held to.
fn install_now(m: &dyn Machine) -> Option<Answer> {
    match m.install() {
        Ok(()) => Some(Answer::Installed),
        Err(e) => {
            eprintln!(
                "note: set up, but the background service could not be installed ({e}); run \
                 `nxs sync daemon install` to retry"
            );
            None
        }
    }
}

/// The summary's `service:` line — pure, so every wording is unit-testable without a terminal.
///
/// It reports the two facts a reader cannot see for themselves and would otherwise have to guess
/// at: whether a service exists on this machine, and whether it knows about THIS workspace. The
/// two are independent — registering is a file this workspace is written into, installing is an
/// agent on the machine — and reporting only one of them is how "no clock here" stayed invisible.
pub fn summary_line(report: &Report) -> String {
    let service = match report.installed {
        Some(true) => "installed".to_string(),
        Some(false) => "not installed — `nxs sync daemon install` sets it up".to_string(),
        None => "no installer on this platform — run `nxs sync daemon` under your own supervisor"
            .to_string(),
    };
    let attends = if report.registered {
        "attends this workspace"
    } else {
        "does NOT know this workspace (its list could not be written)"
    };
    format!("{service}; {attends}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nxs_foundation::error::NxfError;
    use std::cell::RefCell;
    use tempfile::TempDir;

    /// A [`Machine`] that touches nothing: it RECORDS what was asked of it and answers from
    /// fields the test sets, so every branch of [`apply`] is provable without a launchd agent,
    /// a `~/.nexusflow` or a terminal.
    struct Spy {
        installed: RefCell<Option<bool>>,
        install_fails: bool,
        register_fails: bool,
        answer: bool,
        installs: RefCell<u32>,
        registered: RefCell<Vec<String>>,
        asks: RefCell<u32>,
    }

    impl Spy {
        fn new() -> Spy {
            Spy {
                installed: RefCell::new(Some(false)),
                install_fails: false,
                register_fails: false,
                answer: true,
                installs: RefCell::new(0),
                registered: RefCell::new(Vec::new()),
                asks: RefCell::new(0),
            }
        }
    }

    impl Machine for Spy {
        fn installed(&self) -> Option<bool> {
            *self.installed.borrow()
        }
        fn install(&self) -> Result<()> {
            *self.installs.borrow_mut() += 1;
            if self.install_fails {
                return Err(NxfError::io("launchctl said no"));
            }
            *self.installed.borrow_mut() = Some(true);
            Ok(())
        }
        fn register(&self, root: &Path) -> Result<bool> {
            if self.register_fails {
                return Err(NxfError::io("the registry is not writable"));
            }
            self.registered
                .borrow_mut()
                .push(root.display().to_string());
            Ok(true)
        }
        fn ask(&self) -> Result<bool> {
            *self.asks.borrow_mut() += 1;
            Ok(self.answer)
        }
    }

    /// A real workspace on disk, so `apply`'s recording goes through the same `config.toml` the
    /// product does.
    fn workspace() -> (TempDir, Workspace) {
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        (tmp, ws)
    }

    /// Re-read the workspace from disk — what a SECOND `nxs init` would see.
    fn reread(tmp: &TempDir) -> Workspace {
        workspace::discover(tmp.path()).unwrap()
    }

    /// The base case: a real terminal on a supported platform that has never been asked.
    fn fresh() -> Offer {
        Offer {
            supported: true,
            flag: None,
            installed: false,
            answered: false,
            interactive: true,
        }
    }

    #[test]
    fn a_terminal_that_has_never_been_asked_is_asked() {
        assert_eq!(plan(fresh()), Plan::Ask);
    }

    #[test]
    fn a_non_interactive_run_is_never_asked_and_installs_nothing() {
        // `--json` and a pipe are the same case, and it is the one the DoD names twice.
        assert_eq!(
            plan(Offer {
                interactive: false,
                ..fresh()
            }),
            Plan::Skip
        );
    }

    #[test]
    fn the_flag_installs_without_a_terminal() {
        // The non-interactive way in, in the pattern of `--from-beads`.
        assert_eq!(
            plan(Offer {
                flag: Some(true),
                interactive: false,
                ..fresh()
            }),
            Plan::Install
        );
    }

    #[test]
    fn the_flag_installs_on_a_terminal_too_rather_than_asking_what_it_already_said() {
        assert_eq!(
            plan(Offer {
                flag: Some(true),
                ..fresh()
            }),
            Plan::Install
        );
    }

    #[test]
    fn the_negative_flag_records_a_no_without_asking() {
        assert_eq!(
            plan(Offer {
                flag: Some(false),
                ..fresh()
            }),
            Plan::Decline
        );
    }

    #[test]
    fn a_workspace_that_has_already_answered_is_not_asked_again() {
        // The idempotence the ticket names: `init` is re-runnable, and a "no" stays a "no".
        assert_eq!(
            plan(Offer {
                answered: true,
                ..fresh()
            }),
            Plan::Skip
        );
    }

    #[test]
    fn an_already_installed_service_is_not_offered_again() {
        // Nothing to decide: the machine has it. Asking would invite a "no" that changes nothing.
        assert_eq!(
            plan(Offer {
                installed: true,
                ..fresh()
            }),
            Plan::Skip
        );
    }

    #[test]
    fn the_flag_still_reinstalls_over_an_existing_service() {
        // `nxs sync daemon install` is idempotent and re-points the alias; an explicit `--service`
        // is a request to do exactly that, not a question already answered.
        assert_eq!(
            plan(Offer {
                flag: Some(true),
                installed: true,
                ..fresh()
            }),
            Plan::Install
        );
    }

    #[test]
    fn a_platform_without_an_installer_is_not_asked_a_question_it_cannot_answer() {
        assert_eq!(
            plan(Offer {
                supported: false,
                ..fresh()
            }),
            Plan::Skip
        );
    }

    #[test]
    fn a_platform_without_an_installer_answers_an_explicit_request_by_name() {
        // The other half of the DoD's "not asked OR answered by name": silence is right for a
        // question nobody asked, and wrong for a flag somebody typed.
        assert_eq!(
            plan(Offer {
                supported: false,
                flag: Some(true),
                ..fresh()
            }),
            Plan::Unsupported
        );
    }

    #[test]
    fn a_platform_without_an_installer_has_no_no_worth_recording() {
        assert_eq!(
            plan(Offer {
                supported: false,
                flag: Some(false),
                ..fresh()
            }),
            Plan::Skip
        );
    }

    // ---- the recorded answer ----------------------------------------------------------------

    #[test]
    fn an_answer_round_trips_through_the_workspace_config() {
        let (tmp, ws) = workspace();
        assert_eq!(
            recorded(&ws.config),
            None,
            "a fresh workspace has answered nothing"
        );

        assert!(record(&ws.dir, &Answer::Declined).unwrap());
        assert_eq!(recorded(&reread(&tmp).config), Some(Answer::Declined));

        assert!(record(&ws.dir, &Answer::Installed).unwrap());
        assert_eq!(recorded(&reread(&tmp).config), Some(Answer::Installed));
    }

    #[test]
    fn a_hand_written_answer_settles_the_question_and_is_reported_verbatim() {
        // `config.toml` is hand-editable. A word this binary does not know still means somebody
        // answered here, so `init` must not ask again — and must not pretend to understand it.
        let (tmp, ws) = workspace();
        workspace::set_product_key(&ws.dir, SECTION, KEY, "ask-me-in-june".into()).unwrap();
        let answer = recorded(&reread(&tmp).config).expect("a written value is an answer");
        assert_eq!(answer, Answer::Other("ask-me-in-june".to_string()));
        assert_eq!(answer.as_str(), "ask-me-in-june");
    }

    #[test]
    fn a_value_that_is_not_a_word_at_all_is_no_answer() {
        let (tmp, ws) = workspace();
        workspace::set_product_key(&ws.dir, SECTION, KEY, toml::Value::Boolean(true)).unwrap();
        assert_eq!(
            recorded(&reread(&tmp).config),
            None,
            "an unreportable value is not an answer to hide behind"
        );
    }

    // ---- what `apply` does ---------------------------------------------------------------------

    #[test]
    fn every_init_puts_the_workspace_on_the_list_the_service_attends() {
        // The DoD: a workspace that only wants the clock is registered without being bound to a
        // stream. It holds for the NON-interactive path too, which is the one an agent takes.
        let (tmp, ws) = workspace();
        let spy = Spy::new();
        let report = apply(&spy, &ws, None, false);
        assert!(report.registered);
        assert_eq!(
            spy.registered.borrow().as_slice(),
            &[tmp.path().display().to_string()],
            "the PROJECT root is registered, not the `.nxs` directory"
        );
        assert_eq!(*spy.installs.borrow(), 0, "and nothing was installed");
        assert_eq!(*spy.asks.borrow(), 0, "and nothing was asked");
    }

    #[test]
    fn a_registry_that_cannot_be_written_is_reported_rather_than_failing_the_init() {
        let (_tmp, ws) = workspace();
        let spy = Spy {
            register_fails: true,
            ..Spy::new()
        };
        let report = apply(&spy, &ws, None, false);
        assert!(!report.registered, "and it says so instead of claiming it");
    }

    #[test]
    fn a_yes_on_a_terminal_installs_the_service_and_records_it() {
        let (tmp, ws) = workspace();
        let spy = Spy::new();
        let report = apply(&spy, &ws, None, true);
        assert_eq!(*spy.asks.borrow(), 1);
        assert_eq!(*spy.installs.borrow(), 1);
        assert_eq!(report.installed, Some(true));
        assert_eq!(report.answer, Some(Answer::Installed));
        assert_eq!(recorded(&reread(&tmp).config), Some(Answer::Installed));
    }

    #[test]
    fn a_no_on_a_terminal_installs_nothing_and_records_the_no() {
        let (tmp, ws) = workspace();
        let spy = Spy {
            answer: false,
            ..Spy::new()
        };
        let report = apply(&spy, &ws, None, true);
        assert_eq!(*spy.installs.borrow(), 0);
        assert_eq!(report.installed, Some(false));
        assert_eq!(report.answer, Some(Answer::Declined));
        assert_eq!(
            recorded(&reread(&tmp).config),
            Some(Answer::Declined),
            "a no must stay a no until somebody changes it"
        );
    }

    #[test]
    fn a_second_init_in_the_same_workspace_does_not_ask_again() {
        // The DoD's idempotence, driven the way a person meets it: answer once, run again.
        let (tmp, ws) = workspace();
        let first = Spy {
            answer: false,
            ..Spy::new()
        };
        apply(&first, &ws, None, true);
        assert_eq!(*first.asks.borrow(), 1);

        let second = Spy {
            answer: false,
            ..Spy::new()
        };
        apply(&second, &reread(&tmp), None, true);
        assert_eq!(*second.asks.borrow(), 0, "the recorded no is the answer");
        assert_eq!(*second.installs.borrow(), 0);
    }

    #[test]
    fn an_install_that_fails_records_nothing_so_the_next_run_can_offer_again() {
        let (tmp, ws) = workspace();
        let spy = Spy {
            install_fails: true,
            ..Spy::new()
        };
        let report = apply(&spy, &ws, None, true);
        assert_eq!(*spy.installs.borrow(), 1, "it was attempted");
        assert_eq!(report.installed, Some(false));
        assert_eq!(report.answer, None);
        assert_eq!(
            recorded(&reread(&tmp).config),
            None,
            "an install that did not happen is not an answer to record"
        );
    }

    #[test]
    fn a_machine_that_already_runs_the_service_is_not_asked_about_it() {
        let (tmp, ws) = workspace();
        let spy = Spy {
            installed: RefCell::new(Some(true)),
            ..Spy::new()
        };
        let report = apply(&spy, &ws, None, true);
        assert_eq!(*spy.asks.borrow(), 0);
        assert_eq!(report.installed, Some(true));
        assert_eq!(report.answer, None, "nothing was decided here to record");
        assert!(report.registered, "but the workspace is still registered");
        assert_eq!(recorded(&reread(&tmp).config), None);
    }

    #[test]
    fn the_flag_installs_without_a_terminal_and_records_it() {
        let (tmp, ws) = workspace();
        let spy = Spy::new();
        let report = apply(&spy, &ws, Some(true), false);
        assert_eq!(*spy.asks.borrow(), 0, "a flag is an answer, not a question");
        assert_eq!(*spy.installs.borrow(), 1);
        assert_eq!(report.answer, Some(Answer::Installed));
        assert_eq!(recorded(&reread(&tmp).config), Some(Answer::Installed));
    }

    #[test]
    fn the_negative_flag_records_the_no_without_a_terminal() {
        let (tmp, ws) = workspace();
        let spy = Spy::new();
        let report = apply(&spy, &ws, Some(false), false);
        assert_eq!(*spy.installs.borrow(), 0);
        assert_eq!(report.answer, Some(Answer::Declined));
        assert_eq!(recorded(&reread(&tmp).config), Some(Answer::Declined));
    }

    #[test]
    fn an_explicit_request_on_a_platform_with_no_installer_installs_nothing_and_records_nothing() {
        // `Plan::Unsupported` was proven only at the pure `plan` level (review of PR #419, Test
        // Quality #2). This is the arm through `apply`: a flag somebody typed gets a named answer
        // on stderr, and — the part only this level can show — nothing is installed, nothing is
        // recorded, and the workspace is still registered.
        let (tmp, ws) = workspace();
        let spy = Spy {
            installed: RefCell::new(None),
            ..Spy::new()
        };
        let report = apply(&spy, &ws, Some(true), false);
        assert_eq!(
            *spy.installs.borrow(),
            0,
            "there is nothing here to install"
        );
        assert_eq!(*spy.asks.borrow(), 0);
        assert_eq!(report.installed, None);
        assert_eq!(
            report.answer, None,
            "a request that could not be carried out is not an answer to hold a re-run to"
        );
        assert!(report.registered);
        assert_eq!(recorded(&reread(&tmp).config), None);
    }

    #[test]
    fn a_platform_with_no_installer_registers_the_workspace_and_asks_nothing() {
        let (tmp, ws) = workspace();
        let spy = Spy {
            installed: RefCell::new(None),
            ..Spy::new()
        };
        let report = apply(&spy, &ws, None, true);
        assert!(
            report.registered,
            "the registry is a file, and files work everywhere"
        );
        assert_eq!(*spy.asks.borrow(), 0);
        assert_eq!(*spy.installs.borrow(), 0);
        assert_eq!(report.installed, None);
        assert_eq!(recorded(&reread(&tmp).config), None);
    }

    // ---- the line a human reads ------------------------------------------------------------

    fn line(registered: bool, installed: Option<bool>, answer: Option<Answer>) -> String {
        summary_line(&Report {
            registered,
            installed,
            answer,
        })
    }

    #[test]
    fn the_summary_says_the_service_is_installed_and_attending_this_workspace() {
        let s = line(true, Some(true), Some(Answer::Installed));
        assert!(s.contains("installed"), "{s}");
        assert!(
            s.contains("this workspace"),
            "the registration is the half a reader cannot see otherwise: {s}"
        );
    }

    #[test]
    fn the_summary_names_the_command_that_installs_it_later() {
        // Declining must not be a dead end: the way back has to be in the line that reports it.
        let s = line(true, Some(false), Some(Answer::Declined));
        assert!(s.contains("nxs sync daemon install"), "{s}");
    }

    #[test]
    fn the_summary_says_when_the_workspace_did_not_reach_the_list() {
        let s = line(false, Some(true), None);
        assert!(
            s.to_lowercase().contains("not"),
            "a failed registration must not read like a successful one: {s}"
        );
    }

    #[test]
    fn the_summary_on_a_platform_without_an_installer_names_the_way_to_run_it_anyway() {
        let s = line(true, None, None);
        assert!(s.contains("nxs sync daemon"), "{s}");
        assert!(
            !s.contains("nxs sync daemon install"),
            "there is no installer here — naming one would be the false map: {s}"
        );
    }
}
