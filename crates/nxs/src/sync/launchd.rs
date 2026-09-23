//! `nxs sync daemon install` / `uninstall`: the CLI face of the background agent — of THIS
//! process's instance (nxf 6j6v.gd9p), which on a machine that runs only the production service is
//! the one agent it always was.
//!
//! The agent itself — the label, the plist, the `launchctl` seam, the `nexus-flow` alias and the
//! sweep of the retired per-deadline agents — lives in `nxs_service::launchd` since 6j6v.8see, so an
//! embedding app can install the service without linking this crate. What is left here is
//! presentation: the two verbs, their `--json` shape, and the honest refusal on a platform that has
//! no launchd.

#[cfg(not(target_os = "macos"))]
use crate::error::NxfError;
use crate::error::Result;
#[cfg(target_os = "macos")]
use nxs_service::launchd::{Bootstrapped, HomeRule};

/// `nxs sync daemon install`: install (or re-install, idempotently) the nexus-flow service as a
/// launchd agent for the CURRENT binary.
#[cfg(target_os = "macos")]
pub fn install(json: bool, allow_redirected_home: bool) -> Result<()> {
    // **Which instance is being installed** (nxf 6j6v.gd9p) — resolved once here rather than twice
    // inside, so the receipt names exactly the service that was written.
    let home = nxs_service::ServiceHome::resolve()?;
    // Asked BEFORE launchd is touched, so the sentence describes the decision the install is about
    // to act on rather than a re-derivation after the fact. The sentence itself is
    // `nxs_service`'s, because five paths owe it — see its doc.
    let ignored = nxs_service::ignored_instance_env();
    let installed = nxs_service::launchd::install_for(&home, home_rule(allow_redirected_home))?;
    warn_if_installed_from_a_build_tree();
    let instance = home.instance().name();
    let receipt = receipt(&installed.bootstrapped, &home.instance().label());
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "instance": instance,
                "home": home.root().display().to_string(),
                "plist": installed.plist.display().to_string(),
                "confirmed": receipt.confirmed,
                "state": receipt.state,
                "unconfirmed_because": receipt.unconfirmed_because,
                // `null` when the variable was unset, named production, or was obeyed — see
                // `nxs_service::instance_env_ignored_note`. In the receipt AND on stderr below,
                // because a message only a terminal reader meets is a breadcrumb this repo keeps
                // closing.
                "instance_env_ignored": ignored,
            })
        );
    } else {
        println!(
            "installed the {instance} background service ({}), keeping its state in {}",
            installed.plist.display(),
            home.root().display()
        );
        println!("{}", receipt.line);
    }
    if let Some(note) = &ignored {
        eprintln!("note: {note}");
    }
    Ok(())
}

/// **What the install VERIFIED, not what it attempted** (nxf 6j6v.0yrp), as one value both
/// renderings read.
///
/// `launchctl bootstrap` exiting 0 says the request was ACCEPTED; these fields say whether the job
/// launchd actually holds was read back and found to be this plist's. Two different sentences, and
/// the second is the one a reader wanted all along.
///
/// **Pure, and extracted for exactly that reason** (review of PR #428, Test Quality #1). This is
/// the headline claim of the ticket it comes from, and in its first cut it was assembled inline in
/// the middle of `install` where no test could reach it: a refactor could have inverted `confirmed`
/// or dropped a field with nothing going red.
#[cfg(target_os = "macos")]
struct Receipt {
    confirmed: bool,
    state: Option<String>,
    unconfirmed_because: Option<String>,
    /// The line printed under the "installed …" sentence in human mode.
    line: String,
}

#[cfg(target_os = "macos")]
fn receipt(bootstrapped: &Bootstrapped, label: &str) -> Receipt {
    match bootstrapped {
        Bootstrapped::Confirmed { state } => Receipt {
            confirmed: true,
            state: state.clone(),
            unconfirmed_because: None,
            line: match state {
                Some(state) => format!("launchd holds it under {label} (state {state})"),
                None => format!("launchd holds it under {label}"),
            },
        },
        Bootstrapped::Unconfirmed(why) => Receipt {
            confirmed: false,
            state: None,
            unconfirmed_because: Some(why.clone()),
            line: format!(
                "note: the bootstrap was accepted, but the registration could not be read back to \
                 confirm it ({why}) — `nxs sync daemon status` says what launchd holds"
            ),
        },
    }
}

/// The one place the `--allow-redirected-home` flag becomes the rule the seam understands.
///
/// Spelled once so `install` and `uninstall` cannot disagree about what the flag means, and named
/// so the two call sites read as the decision rather than as a boolean.
#[cfg(target_os = "macos")]
fn home_rule(allow_redirected_home: bool) -> HomeRule {
    if allow_redirected_home {
        HomeRule::RedirectedIsAllowed
    } else {
        HomeRule::MustBeTheLoginSessions
    }
}

/// `nxs sync daemon uninstall`: stop and remove the launchd agent. Not an error if it was never
/// installed.
#[cfg(target_os = "macos")]
pub fn uninstall(json: bool, allow_redirected_home: bool) -> Result<()> {
    let instance = nxs_service::Instance::ambient()?;
    // The same sentence as `install`'s, and here it is the sharper half of it: somebody who
    // believes they are removing a development service and is in fact stopping the machine's clock
    // must be told which one went (nxf 6j6v.cvpy).
    let ignored = nxs_service::ignored_instance_env();
    nxs_service::launchd::uninstall_for(&instance, home_rule(allow_redirected_home))?;
    // The home is deliberately LEFT: it holds the registry a re-install resumes from, and removing
    // somebody's workspace list because they stopped the agent for an afternoon is not what
    // `uninstall` was asked to do.
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "instance": instance.name(),
                "instance_env_ignored": ignored,
            })
        );
    } else {
        println!("uninstalled the {instance} background service");
    }
    if let Some(note) = &ignored {
        eprintln!("note: {note}");
    }
    Ok(())
}

/// Say so when the agent has just been pointed at a binary inside a build tree.
///
/// The agent runs whatever `current_exe()` was at install time, through the `nexus-flow` link. That
/// is right for an installed binary and a trap for a developer's `target/debug` one: `cargo clean`,
/// a rename, or a branch switch leaves the service pointing at nothing, and launchd's only report of
/// that is a service that never starts. The retired per-deadline agents had the same shape as a
/// `PATH` entry, and it is named in 6j6v.8see as complaint #4.
///
/// A note, not a refusal: installing from a build tree is exactly what a developer testing this
/// wants to do — and since nxf 6j6v.cvpy it is the ONLY way to install a development instance.
/// What they should not have is a silent one.
///
/// The judgement is `nxs_service::Origin`'s, which is the same one that decided WHICH instance was
/// just installed: two spellings of "is this a build" could disagree, and the one that decides an
/// instance must be the one that describes it.
#[cfg(target_os = "macos")]
fn warn_if_installed_from_a_build_tree() {
    let Some(exe) = nxs_service::program::running_program() else {
        return;
    };
    if nxs_service::Origin::of(&exe) == nxs_service::Origin::Build {
        eprintln!(
            "note: the service now runs {}, which is inside a build tree — `cargo clean` or a \
             moved checkout will leave it pointing at nothing. Re-run `nxs sync daemon install` \
             from an installed binary when you are done.",
            exe.display()
        );
    }
}

/// The bind-time autostart: no output, propagates `Err` — the caller (`sync::bind`'s `Autostart`
/// seam) treats installation as a best-effort side effect of a successful bind and only warns on
/// failure, never fails the bind itself.
///
/// **The verification verdict is dropped here on purpose** (nxf 6j6v.0yrp). `install` above turns
/// it into a line of the receipt; this path has no receipt to put it in, and an unconfirmed
/// bootstrap is not a failure — it is an install nobody could read back, which is exactly what
/// `nxs sync daemon status` is for. What is NOT dropped is the `Err`: a bootstrap that failed all
/// five attempts still reaches `bind`'s warning with the precondition it named.
#[cfg(target_os = "macos")]
pub fn install_quiet() -> Result<()> {
    nxs_service::launchd::install().map(|_| ())
}

/// Say so when this workspace has just become the second instance's to attend (nxf 6j6v.gd9p).
///
/// **Platform-free**, unlike everything above it: registries are files, and two of them on a Linux
/// machine overlap exactly as they do on a Mac. The sentence comes from `nxs_service` so `bind`,
/// `status` and the running service's own log cannot describe the same overlap differently.
///
/// A note, not a refusal, for [`warn_if_installed_from_a_build_tree`]'s reason: two instances
/// attending one workspace is a thing a person may deliberately be doing for an afternoon. What
/// they should not have is a silent one.
pub fn warn_if_attended_by_more_than_one() {
    let Ok(home) = nxs_service::ServiceHome::resolve() else {
        return;
    };
    // The note itself is `nxs_init::service::overlap_note` (nxf 6j6v.y12q): since the module front
    // doors register too, three paths write this registry and all three owe the same sentence —
    // so it is spelled once, over there, where the shared registration seam lives.
    if let Some(note) = nxs_init::service::overlap_note(&home) {
        eprintln!("note: {note}");
    }
}

/// The message every non-macOS arm gives. One string, so the three cannot drift.
#[cfg(not(target_os = "macos"))]
fn macos_only(verb: &str) -> NxfError {
    NxfError::validation(format!(
        "`{verb}` manages a launchd agent and is macOS-only. On this platform run \
         `nxs sync daemon` in the foreground (that is exactly what launchd starts on macOS), e.g. \
         from systemd, runit, or your own supervisor — the service behaves identically there, \
         including the deadlines it keeps."
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn install(_json: bool, _allow_redirected_home: bool) -> Result<()> {
    Err(macos_only("nxs sync daemon install"))
}

#[cfg(not(target_os = "macos"))]
pub fn uninstall(_json: bool, _allow_redirected_home: bool) -> Result<()> {
    Err(macos_only("nxs sync daemon uninstall"))
}

/// Same platform gate as [`install`]/[`uninstall`] — the bind-time autostart is exactly as
/// macOS-only as the explicit verbs it retries via, so `sync::bind`'s `Autostart` seam gets a loud
/// reason (swallowed into a warning there, same as any other install failure) rather than a silent
/// no-op.
#[cfg(not(target_os = "macos"))]
pub fn install_quiet() -> Result<()> {
    Err(macos_only("the nexus-flow launchd agent"))
}

#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
    use super::*;

    /// **6j6v.0yrp's headline claim, at the surface that carries it** (review of PR #428, Test
    /// Quality #1). A confirmed install says so, in the receipt AND in the line — and `state` is
    /// launchd's own word for what it is doing, carried through rather than invented here.
    #[test]
    fn a_confirmed_install_reports_the_state_launchd_read_back() {
        let r = receipt(
            &Bootstrapped::Confirmed {
                state: Some("running".to_string()),
            },
            "com.nxsflow.nexus-flow",
        );
        assert!(r.confirmed);
        assert_eq!(r.state.as_deref(), Some("running"));
        assert_eq!(r.unconfirmed_because, None);
        assert!(r.line.contains("com.nxsflow.nexus-flow"), "{}", r.line);
        assert!(r.line.contains("running"), "{}", r.line);
    }

    /// launchd answered, and said nothing about the state. The receipt still confirms — the
    /// verification is about WHOSE job is loaded, and that question was answered.
    #[test]
    fn a_confirmed_install_with_no_state_still_confirms_and_says_nothing_it_does_not_know() {
        let r = receipt(&Bootstrapped::Confirmed { state: None }, "com.nxsflow.x");
        assert!(r.confirmed);
        assert_eq!(r.state, None);
        assert_eq!(r.line, "launchd holds it under com.nxsflow.x");
    }

    /// The arm this whole extraction is for: a bootstrap that was accepted and never read back is
    /// NOT a confirmed install, and inverting that boolean must not be something a refactor can do
    /// in silence.
    #[test]
    fn an_unread_registration_is_reported_as_unconfirmed_with_its_reason() {
        let r = receipt(
            &Bootstrapped::Unconfirmed("launchctl print exited with 1: fork failed".to_string()),
            "com.nxsflow.x",
        );
        assert!(
            !r.confirmed,
            "an install nobody read back is not a confirmed one"
        );
        assert_eq!(r.state, None, "and it must not invent one");
        assert!(r
            .unconfirmed_because
            .as_deref()
            .is_some_and(|w| w.contains("fork failed")));
        assert!(r.line.contains("could not be read back"), "{}", r.line);
        assert!(
            r.line.contains("nxs sync daemon status"),
            "and where to get the answer instead: {}",
            r.line
        );
    }

    /// The flag is a rule with two values and no third reading.
    #[test]
    fn the_flag_maps_to_exactly_one_rule_each_way() {
        assert_eq!(home_rule(false), HomeRule::MustBeTheLoginSessions);
        assert_eq!(home_rule(true), HomeRule::RedirectedIsAllowed);
    }
}
