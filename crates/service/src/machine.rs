//! **This machine** — the name a person picks it by, and the id the relay knows it by (nxf 6j6v.f0b5).
//!
//! # What a "machine" is
//!
//! A machine is a SERVICE HOME: the directory one background-service instance keeps its state in
//! ([`ServiceHome`]). On an ordinary computer there is exactly one — the production `nexus-flow`
//! service in `~/.nexusflow` — so a machine is the computer. A computer that also runs a
//! development instance (`~/.nexusflow-dev`, 6j6v.gd9p) is two machines, and that is decided, not
//! incidental:
//!
//! - **The one that executes is a service.** The second slice of this work (6j6v.1c6k) lets a
//!   person hand a chat to a named machine, and what picks it up is that machine's service after
//!   its next pull. Two instances sharing one identity would BOTH take it — their locks, registries
//!   and clocks are separate by construction, which is the whole point of gd9p. One identity per
//!   home makes the machine a work order names exactly one executor.
//! - **Instances are already isolated everywhere else.** Each has its own registry and its own
//!   endpoint configuration — it may not even sync to the same relay. An identity shared across
//!   them would be the one piece of state that leaks between homes, which gd9p calls worse than
//!   none.
//! - **A person can still tell them apart.** A development instance's default name carries its
//!   qualifier (`studio (dev)`), so a list never shows two indistinguishable entries.
//!
//! # The file
//!
//! `<home>/machine.toml`: an id (a lowercase ULID minted here — random but for its first ten
//! characters, which say when it was minted; never derived from hardware and never the update
//! channel's rollout id, so the relay and the update server cannot link one install across both)
//! and a name. Minted on the first ask, then durable: a machine that
//! got a new id would appear twice in every list, the old one fading to "not online" as if it had
//! gone away. That is also why a file that cannot be read is refused instead of minted over.
//!
//! # The name
//!
//! The default is the short host name — the name the computer already announces on every local
//! network — plus the instance qualifier where there is one. It is shown to anybody who can read
//! the stream at a relay that authenticates nobody (see `nxs_sync::presence`), so it is the owner's
//! to change: [`ServiceHome::rename_machine`], `nxs sync machine <name>` on the command line.

use std::path::PathBuf;

use nxs_foundation::error::{NxfError, Result};
use serde::{Deserialize, Serialize};

use crate::home::ServiceHome;
use crate::instance::Instance;

/// The file inside a service home.
const MACHINE_FILE: &str = "machine.toml";

/// The longest name, in characters — the same bound the relay stores
/// (`nxs_sync::presence::MAX_MACHINE_NAME_CHARS`; `crates/nxs` holds a test that the two agree), so
/// a rename this accepts is never refused by a relay later.
pub const MAX_NAME_CHARS: usize = 64;

/// This machine's identity: what a relay lists and what a person chooses by.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Machine {
    pub id: String,
    pub name: String,
}

impl ServiceHome {
    /// `<home>/machine.toml` — this service home's machine identity.
    pub fn machine_file(&self) -> PathBuf {
        self.root().join(MACHINE_FILE)
    }

    /// This machine, minting it on the first ask. Concurrent first asks — the service and a verb
    /// starting at the same moment — agree on ONE identity: the mint only ever creates the file,
    /// it never replaces one, and the loser reads what the winner wrote.
    pub fn machine(&self) -> Result<Machine> {
        if let Some(machine) = self.peek_machine()? {
            return Ok(machine);
        }
        let id = ulid::Ulid::new().to_string().to_ascii_lowercase();
        let machine = Machine {
            name: default_name(host_name().as_deref(), self.instance(), &id),
            id,
        };
        if create_new(&self.machine_file(), &render(&machine)?, &machine.id)? {
            return Ok(machine);
        }
        self.peek_machine()?.ok_or_else(|| {
            NxfError::io(format!(
                "{} vanished while it was being created",
                self.machine_file().display()
            ))
        })
    }

    /// This machine if it has been minted — never mints. What a read that must leave no trace
    /// asks (listing a stream's machines marks this one, but must not create it to do so).
    ///
    /// A file that does not parse, or parses into an id or name no relay would accept, is refused
    /// with the way out rather than minted over or announced into refusal on every pass.
    pub fn peek_machine(&self) -> Result<Option<Machine>> {
        let path = self.machine_file();
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(NxfError::io(format!("reading {}: {e}", path.display()))),
        };
        let unusable = |why: String| {
            NxfError::validation(format!(
                "{} is not a usable machine file ({why}) — refusing to mint a new identity over it. \
                 Fix it by hand, or delete it to give this machine a NEW identity (every relay will \
                 then list the old one as offline until it expires)",
                path.display()
            ))
        };
        let machine: Machine = toml::from_str(&raw).map_err(|e| unusable(e.to_string()))?;
        check_id(&machine.id).map_err(unusable)?;
        check_name(&machine.name).map_err(unusable)?;
        Ok(Some(machine))
    }

    /// Give this machine a new name; the id stays. Surrounding whitespace is dropped; a name that
    /// is blank, longer than [`MAX_NAME_CHARS`] or carries a control character is refused with
    /// nothing changed. The service picks it up on its next pass, without a restart.
    pub fn rename_machine(&self, name: &str) -> Result<Machine> {
        let name = name.trim();
        check_name(name).map_err(NxfError::validation)?;
        let mut machine = self.machine()?;
        machine.name = name.to_string();
        crate::atomic::write_atomic_unique(&self.machine_file(), render(&machine)?.as_bytes())?;
        Ok(machine)
    }
}

/// The rule a machine's name is held to — the same one the relay holds an announcement to
/// (`nxs_sync::presence::check_machine_name`; `crates/nxs` pins the two together, since this crate
/// sits below the embedding closure and may not link `nxs-sync`). A person picks a machine by
/// reading it, so the name must render as what it is: at least one visible character, no control
/// character, no whitespace but the plain space, and none of the characters that render as nothing
/// or reorder the display (the house set of `nxs_foundation::model::is_attributable`).
pub fn check_name(name: &str) -> std::result::Result<(), String> {
    if name.trim().is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || !name.chars().all(renders_as_itself)
    {
        return Err(format!(
            "a machine name must be 1 to {MAX_NAME_CHARS} characters, not blank, with no control, \
             invisible or direction-changing characters and no whitespace but spaces"
        ));
    }
    Ok(())
}

/// The rule a machine's id is held to — the relay's (`nxs_sync::presence::check_id`): 1 to 64
/// printable ASCII characters, no space. A minted id always passes; this catches a hand edit.
fn check_id(id: &str) -> std::result::Result<(), String> {
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("the id must be 1 to 64 printable ASCII characters without spaces".into());
    }
    Ok(())
}

/// Whether `c` may stand in a machine name: see [`check_name`].
fn renders_as_itself(c: char) -> bool {
    !c.is_control()
        && (c == ' ' || !c.is_whitespace())
        && !matches!(c,
            '\u{00AD}'                 // soft hyphen
            | '\u{061C}'               // arabic letter mark
            | '\u{180E}'               // mongolian vowel separator
            | '\u{200B}'..='\u{200F}'  // zero-width space/non-joiner/joiner, LRM, RLM
            | '\u{202A}'..='\u{202E}'  // bidi embedding + override
            | '\u{2060}'..='\u{2064}'  // word joiner, invisible operators
            | '\u{2066}'..='\u{2069}'  // bidi isolates
            | '\u{FEFF}'               // zero-width no-break space (BOM)
        )
}

/// The name a freshly minted machine gets: the host name up to its first dot (`studio.local` →
/// `studio`), with the instance qualifier for a development instance (`studio (dev)`). Without a
/// usable host name it is built from the id, so it is never blank. Always within
/// [`check_name`] — a host name is shortened, never refused.
pub fn default_name(host: Option<&str>, instance: &Instance, id: &str) -> String {
    let short: String = host
        .and_then(|h| h.split('.').next())
        .map(|h| {
            h.chars()
                .filter(|&c| renders_as_itself(c))
                .collect::<String>()
        })
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| format!("machine-{}", id.chars().take(6).collect::<String>()));
    let suffix = instance
        .qualifier()
        .map(|q| format!(" ({q})"))
        .unwrap_or_default();
    let room = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
    let base: String = short.chars().take(room).collect();
    format!("{}{suffix}", base.trim_end())
}

/// This computer's host name, from the platform. `None` where it cannot be read.
fn host_name() -> Option<String> {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: `buf` is a valid, writable buffer of the length passed, and `gethostname` writes
        // at most that many bytes into it.
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc != 0 {
            return None;
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Some(String::from_utf8_lossy(&buf[..end]).into_owned())
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

fn render(machine: &Machine) -> Result<String> {
    toml::to_string(machine).map_err(|e| NxfError::io(format!("serializing the machine file: {e}")))
}

/// Create `path` with `contents` only if nothing is there yet. Returns whether THIS call created
/// it. The contents go to a private temp first, are flushed to disk, and are then hard-linked into
/// place — a link, unlike a rename, refuses an existing target — so a reader never sees a
/// half-written file, a crash never leaves an empty one, and two concurrent creators never
/// overwrite each other.
///
/// The temp is named by `unique` (the freshly minted id) as well as the pid: two threads of ONE
/// process — an embedding host asking from two places at once — share a pid, and a pid-only name
/// made the second one fail on the first one's temp.
pub(crate) fn create_new(path: &std::path::Path, contents: &str, unique: &str) -> Result<bool> {
    create_new_with(path, contents, unique, |from, to| {
        std::fs::hard_link(from, to)
    })
}

/// [`create_new`] with the link step injectable, so the fallback below is provable on a file
/// system that HAS hard links.
///
/// Where the volume refuses links altogether (exFAT, FAT, some network shares), it falls back to an
/// exclusive create of the target itself: still never an overwrite, only without the guarantee that
/// a concurrent reader cannot catch the file half-written — which `peek_machine` then refuses
/// loudly rather than misreads.
fn create_new_with(
    path: &std::path::Path,
    contents: &str,
    unique: &str,
    link: impl Fn(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
) -> Result<bool> {
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(dir)
        .map_err(|e| NxfError::io(format!("creating {}: {e}", dir.display())))?;
    let tmp = dir.join(format!(
        ".{MACHINE_FILE}.nxs.new.{}.{unique}",
        std::process::id()
    ));
    let written = crate::atomic::write_new_exclusive(&tmp, contents.as_bytes())
        .and_then(|()| std::fs::File::open(&tmp)?.sync_all());
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(NxfError::io(format!("writing {}: {e}", tmp.display())));
    }
    let linked = link(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(_) => match crate::atomic::write_new_exclusive(path, contents.as_bytes()) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(NxfError::io(format!("creating {}: {e}", path.display()))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Instance;
    use tempfile::TempDir;

    fn home() -> (TempDir, ServiceHome) {
        let dir = TempDir::new().unwrap();
        let home = ServiceHome::at(dir.path().join(".nexusflow"));
        (dir, home)
    }

    #[test]
    fn the_first_ask_mints_an_id_and_a_name_and_every_later_ask_returns_the_same() {
        let (_dir, home) = home();
        let first = home.machine().unwrap();
        assert_eq!(first.id.len(), 26, "a ULID: {first:?}");
        assert!(!first.name.trim().is_empty(), "{first:?}");
        assert!(home.machine_file().is_file());
        assert_eq!(home.machine().unwrap(), first, "minted once, then durable");
    }

    #[test]
    fn peeking_never_mints() {
        let (_dir, home) = home();
        assert_eq!(home.peek_machine().unwrap(), None);
        assert!(!home.machine_file().exists(), "a read left a file behind");
        let minted = home.machine().unwrap();
        assert_eq!(home.peek_machine().unwrap(), Some(minted));
    }

    #[test]
    fn concurrent_first_asks_agree_on_one_identity() {
        // The service and a verb starting at the same moment on a fresh machine: a mint that
        // REPLACED the file would leave one of them announcing an id nobody else ever sees again.
        let (_dir, home) = home();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (home, barrier) = (home.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    home.machine().unwrap()
                })
            })
            .collect();
        let seen: std::collections::HashSet<Machine> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert_eq!(home.machine().unwrap(), seen.into_iter().next().unwrap());
    }

    #[test]
    fn a_rename_keeps_the_id_and_survives_a_reread() {
        let (_dir, home) = home();
        let before = home.machine().unwrap();
        let after = home.rename_machine("Mac mini").unwrap();
        assert_eq!(after.id, before.id, "the id is what other machines know");
        assert_eq!(after.name, "Mac mini");
        assert_eq!(home.machine().unwrap(), after);
    }

    #[test]
    fn renaming_a_machine_that_was_never_asked_about_mints_it_under_the_new_name() {
        let (_dir, home) = home();
        let machine = home.rename_machine("  Mac mini  ").unwrap();
        assert_eq!(
            machine.name, "Mac mini",
            "surrounding space is not part of a name"
        );
        assert_eq!(machine.id.len(), 26);
    }

    #[test]
    fn a_name_nobody_could_read_in_a_list_is_refused_and_nothing_changes() {
        let (_dir, home) = home();
        let before = home.machine().unwrap();
        for bad in ["", "   ", "tab\there", &"x".repeat(MAX_NAME_CHARS + 1)] {
            let err = home.rename_machine(bad).unwrap_err();
            assert_eq!(
                err.kind,
                nxs_foundation::error::ErrorKind::Validation,
                "{bad:?}"
            );
        }
        assert_eq!(home.machine().unwrap(), before);
        // The bound counts characters, not bytes.
        assert!(home.rename_machine(&"é".repeat(MAX_NAME_CHARS)).is_ok());
    }

    #[test]
    fn a_name_that_renders_as_something_else_than_it_is_refused_at_rename() {
        // The same rule the relay holds an announcement to (`nxs_sync::presence::check_machine_name`),
        // so a rename that passes here never becomes a machine every relay silently refuses.
        let (_dir, home) = home();
        for bad in [
            "\u{200B}",
            "Mac\u{202E}mini",
            "\u{FEFF}Mac",
            "Mac\u{00A0}mini",
        ] {
            assert!(home.rename_machine(bad).is_err(), "{bad:?} must be refused");
        }
        assert!(home.rename_machine("Carsten’s Mac mini (dev)").is_ok());
    }

    #[test]
    fn a_machine_file_that_parses_but_breaks_the_rules_is_refused_with_the_way_out() {
        // A hand-edited file with an id or name no relay accepts would otherwise be announced — and
        // refused — on every pass, while `nxs sync machine` showed nothing wrong.
        let (_dir, home) = home();
        std::fs::create_dir_all(home.root()).unwrap();
        for bad in [
            "id = \"has space\"\nname = \"Mac mini\"\n",
            "id = \"01j8zqk7m2n4p6r8s0t2v4w6x8\"\nname = \"Mac\u{202E}mini\"\n",
        ] {
            std::fs::write(home.machine_file(), bad).unwrap();
            let err = home.peek_machine().unwrap_err();
            assert_eq!(
                err.kind,
                nxs_foundation::error::ErrorKind::Validation,
                "{bad:?}"
            );
            assert!(err.msg.contains("machine.toml"), "{}", err.msg);
            assert!(err.msg.contains("delete"), "names the way out: {}", err.msg);
        }
    }

    #[test]
    fn concurrent_renames_in_one_process_all_land() {
        // Review of PR #485, Integrity #5: the shared atomic writer named its temp by pid alone, so
        // two renames from two threads of one embedding host collided on it — the same class the
        // concurrent-mint test found on the mint path.
        let (_dir, home) = home();
        home.machine().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let (home, barrier) = (home.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    home.rename_machine(&format!("Machine {i}"))
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap().expect("every rename succeeds");
        }
        assert!(home.machine().unwrap().name.starts_with("Machine "));
    }

    #[test]
    fn a_file_system_without_hard_links_still_gets_a_machine() {
        // exFAT, FAT and some network volumes refuse `link`. The mint falls back to an exclusive
        // create instead of failing forever (review of PR #485, Code Quality #9).
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("machine.toml");
        let no_links = |_: &std::path::Path, _: &std::path::Path| -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "no hard links here",
            ))
        };
        assert!(create_new_with(&path, "id = \"a\"\nname = \"b\"\n", "u1", no_links).unwrap());
        assert!(
            !create_new_with(&path, "id = \"c\"\nname = \"d\"\n", "u2", no_links).unwrap(),
            "the fallback is exclusive too: a second creator does not overwrite the first"
        );
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"a\""));
    }

    #[test]
    fn a_machine_file_that_cannot_be_read_is_refused_rather_than_minted_over() {
        // Minting over it would silently give this machine a NEW id — every relay would then list
        // it twice, the old one fading to "not online" as if the machine had gone away.
        let (_dir, home) = home();
        std::fs::create_dir_all(home.root()).unwrap();
        std::fs::write(home.machine_file(), "id = [not toml").unwrap();
        let err = home.machine().unwrap_err();
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Validation);
        assert!(err.msg.contains("machine.toml"), "{}", err.msg);
        assert_eq!(
            std::fs::read_to_string(home.machine_file()).unwrap(),
            "id = [not toml",
            "left exactly as it was"
        );
    }

    #[test]
    fn two_service_homes_are_two_machines() {
        // The decision this module documents: a machine is a service home. Two homes on one
        // computer — two instances — are two machines, each with its own id.
        let dir = TempDir::new().unwrap();
        let production = ServiceHome::at(dir.path().join(".nexusflow"));
        let dev = ServiceHome::at_instance(
            dir.path().join(".nexusflow-dev"),
            Instance::named("nexus-flow-dev").unwrap(),
        );
        assert_ne!(production.machine().unwrap().id, dev.machine().unwrap().id);
    }

    #[test]
    fn the_default_name_is_the_short_host_name() {
        let production = Instance::production();
        assert_eq!(
            default_name(Some("Carstens-MacBook-Pro.local"), &production, "01j8"),
            "Carstens-MacBook-Pro"
        );
        assert_eq!(
            default_name(Some("  studio  "), &production, "01j8"),
            "studio"
        );
    }

    #[test]
    fn a_development_instance_names_itself_apart_from_the_machine_s_own_service() {
        let dev = Instance::named("nexus-flow-dev").unwrap();
        assert_eq!(
            default_name(Some("studio.lan"), &dev, "01j8"),
            "studio (dev)"
        );
    }

    #[test]
    fn without_a_host_name_the_default_is_built_from_the_id() {
        let production = Instance::production();
        assert_eq!(
            default_name(None, &production, "01j8zqk7m2n4p6r8s0t2v4w6x8"),
            "machine-01j8zq"
        );
        assert_eq!(
            default_name(Some(" . "), &production, "01j8zqk7m2n4p6r8s0t2v4w6x8"),
            "machine-01j8zq",
            "a host name with nothing before its first dot is no name"
        );
    }

    #[test]
    fn a_default_name_always_passes_the_rule_a_rename_is_held_to() {
        let dev = Instance::named("nexus-flow-foundations-dev").unwrap();
        let long = "h".repeat(200);
        for host in [
            Some(long.as_str()),
            Some("ctl\u{7}host"),
            Some("stu\u{200B}dio\u{202E}"),
            Some("x"),
            None,
        ] {
            let name = default_name(host, &dev, "01j8zqk7m2n4p6r8s0t2v4w6x8");
            assert_eq!(check_name(&name), Ok(()), "{host:?} -> {name:?}");
        }
    }

    #[test]
    fn this_computer_has_a_host_name_to_offer() {
        // Not an assertion about the name — only that the platform call answers, so the default
        // is a readable name and not the id fallback on the machines this ships to.
        if cfg!(any(unix, windows)) {
            assert!(host_name().is_some_and(|h| !h.trim().is_empty()));
        }
    }
}
