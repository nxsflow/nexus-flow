//! **Which nexus-flow service this is** (nxf 6j6v.gd9p).
//!
//! Until this module there was exactly ONE service on a machine, and every one of its identifying
//! marks was a constant: one launchd label, one home directory, one registry, one single-instance
//! lock, one heartbeat, one alias. Whoever installed last owned the machine's clock — for everybody
//! else too. Measured on the owner's machine on 2026-08-29: the alias had pointed at a
//! `target/debug/nxs` since the 25th, and three `nxs self-update` runs on one day had not touched
//! it, because there was nowhere else for it to point.
//!
//! An [`Instance`] is the one value all four derivations now come out of:
//!
//! | instance name                | launchd label                             | home directory        |
//! |------------------------------|-------------------------------------------|-----------------------|
//! | `nexus-flow`                 | `com.nxsflow.nexus-flow`                  | `~/.nexusflow`        |
//! | `nexus-flow-dev`             | `com.nxsflow.nexus-flow-dev`              | `~/.nexusflow-dev`    |
//! | `nexus-flow-foundations-dev` | `com.nxsflow.nexus-flow-foundations-dev`  | `~/.nexusflow-foundations-dev` |
//!
//! The instance name is ALSO the alias's file name ([`ServiceHome::program`](crate::ServiceHome::program)),
//! and that is not decoration — it is the whole mechanism. `render_plist` writes
//! `ProgramArguments` with exactly one element, so a RUNNING service cannot be told which instance
//! it is by a flag; it reads it off its own `argv[0]`, the way `nxs` already answers to
//! `nxf`/`nxm`/`nxc`.
//!
//! # Where the answer comes from
//!
//! [`Instance::resolve_from`] is the whole precedence, pure over its three inputs:
//!
//! 1. **`argv[0]`** — the name this process was started as. The launchd job `exec`s the alias, so
//!    the service reads its own instance out of the name it is running under, with no argument, no
//!    environment and nothing to configure. A person running `~/.nexusflow-dev/bin/nexus-flow-dev
//!    status` by hand gets the same answer for the same reason.
//! 2. **[`Origin`] — where the RUNNING BINARY sits** (nxf 6j6v.cvpy). A binary at its installation
//!    location is the production one, and nothing in the environment can move it. Only a binary
//!    inside a build directory reaches step 3 at all.
//! 3. **`NXS_SERVICE_INSTANCE`** — which development instance a build belongs to, since one machine
//!    holds several working copies. All three repos that run their own instance already use
//!    `direnv`, so a variable in `.envrc` is the introduced way and needs no new mechanism.
//! 4. **Production** otherwise. A machine that has never heard of any of this keeps the service it
//!    already has, under the name and in the directory it already has.
//!
//! # Why step 2 exists at all (nxf 6j6v.cvpy)
//!
//! Because without it the DIRECTORY decided, not the binary — and it decided wrongly for the
//! commonest invocation there is. `NXS_SERVICE_INSTANCE` is exported by `direnv` for a whole
//! working copy, so every call made while standing in it was a call of the development instance,
//! the INSTALLED `nxs`/`nxf`/`nxm`/`nxc` on the `PATH` included. Measured on the owner's machine on
//! 2026-09-05: all three registered services — `nexus-flow`, `nexus-flow-dev`,
//! `nexus-flow-foundations-dev` — pointed at the same `~/.local/bin/nxs`, so the two development
//! services had never run a development build at all, while the everyday `nxf` in the checkout was
//! talking to a development registry it had no business in.
//!
//! The rule that replaces it (owner decision, 2026-09-05) is one sentence: **the running binary
//! decides.** An install takes [`std::env::current_exe`] for the alias it writes, so a development
//! instance installed from a build directory now points at that build BY CONSTRUCTION — which is
//! the half the old arrangement could never reach.
//!
//! # What a name may be
//!
//! A name reaches a FILESYSTEM PATH COMPONENT and a launchd label, and step 3 above takes it from
//! an environment variable. So [`Instance::named`] validates rather than trusts: the base name,
//! optionally followed by `-` and lowercase alphanumeric segments. `..`, `/`, whitespace, an empty
//! qualifier and a trailing dash are all refused loudly — guessing at what somebody meant is how a
//! service ends up keeping its state one directory above the one it was asked to.

use std::path::Path;

use nxs_foundation::error::{NxfError, Result};

/// The production instance's name — and the stem every other instance's name extends.
///
/// It is what the user sees in their macOS background items, so it is the product's name rather
/// than the binary's (`nxs`).
const BASE_NAME: &str = "nexus-flow";

/// The production instance's directory under `$HOME`. Deliberately NOT `.` + [`BASE_NAME`]: the
/// directory has been `.nexusflow`, without the dash, since long before it had a service in it, and
/// renaming it would strand the registry, lock and heartbeat of every machine that already has one.
const BASE_HOME_DIR: &str = ".nexusflow";

/// The reverse-DNS prefix every label this repo installs carries.
const LABEL_PREFIX: &str = "com.nxsflow.";

/// The environment variable a CALLER (not the service) names its instance with — see the module
/// doc's precedence.
pub const INSTANCE_ENV: &str = "NXS_SERVICE_INSTANCE";

/// The longest qualifier a name may carry. Not a technical limit — a launchd label and a directory
/// name both tolerate far more — but a ceiling on how wrong a mis-set environment variable can go
/// before it is refused instead of silently creating a directory nobody meant.
const MAX_QUALIFIER: usize = 48;

/// The two directories Cargo builds into by NAME. Not the whole answer — a `--profile` of one's own
/// is `target/<that name>/` and matches none of these — which is what [`Origin::of`]'s other two
/// questions are for.
const PROFILE_DIRS: [&str; 2] = ["debug", "release"];

/// The directory name Cargo's own layout puts a profile directory inside.
const TARGET_DIR: &str = "target";

/// Cargo writes this file into the target directory, whatever the directory is CALLED — so it is
/// the one thing that can recognise a build tree that no name gives away (a `--profile` of one's
/// own inside a redirected `CARGO_TARGET_DIR`).
const CACHEDIR_TAG: &str = "CACHEDIR.TAG";

/// The first line of that file, from the cache-directory-tagging spec Cargo follows.
const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

/// **Where the running binary sits** — and therefore whether it is the machine's service or a
/// development one (nxf 6j6v.cvpy).
///
/// This is step 2 of the module doc's precedence, and it is the answer to the question the old
/// arrangement never asked: a `PATH` binary standing in a working copy is still the INSTALLED
/// binary, whatever the working copy's `.envrc` exports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A binary at its installation location — `~/.local/bin/nxs`, `~/.cargo/bin/nxs`, a package
    /// manager's `bin` — which is to say: anywhere that is not a build directory. It is the
    /// machine's own suite, so it is the machine's own service: `NXS_SERVICE_INSTANCE` is not read
    /// at all for it, not even to be validated.
    Installed,
    /// A binary inside a Cargo build directory. It belongs to a working copy rather than to the
    /// machine, so `NXS_SERVICE_INSTANCE` gets to say WHICH working copy's instance that is.
    Build,
}

impl Origin {
    /// Judge a path — by three questions, asked in that order, and the order is the cost:
    ///
    /// 1. **Is it in a profile directory?** `<profile>/nxs` and `<profile>/deps/<test>-<hash>` are
    ///    the two shapes Cargo writes, and this catches both, under a redirected
    ///    `CARGO_TARGET_DIR` as well, which drops the word `target` from the path entirely.
    /// 2. **Is it under a `target` directory at all?** This is what catches a `--profile` of one's
    ///    own (`target/release-lto/nxs`), which has no name from list 1 anywhere in it.
    /// 3. **Does an ancestor carry Cargo's own [`CACHEDIR_TAG`]?** The last resort, and the only
    ///    question that survives BOTH of the above being wrong at once — a private profile inside
    ///    a redirected target directory. One `open` per ancestor, and only for a path the first
    ///    two already called installed, so an installed binary pays a handful of `stat`s once.
    ///
    /// **Why three and not one** (review of PR #441, Code Quality #1 / Integrity #1): the first
    /// version asked question 1 alone, and a private profile therefore read as INSTALLED — which
    /// is the one direction that must not be wrong. A false `Build` only leaves the environment
    /// the vote it had before this rule existed; a false `Installed` silently moves a development
    /// build onto the production registry, which is the failure this whole module was written to
    /// end. So the questions are ordered cheap-to-thorough and every one of them can only widen
    /// `Build`.
    ///
    /// It takes the path rather than reading the process: a judgement made about the process is a
    /// judgement no test can drive.
    pub fn of(exe: &Path) -> Origin {
        let named = |dir: Option<&Path>, name: &str| {
            dir.and_then(Path::file_name)
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == name)
        };
        let named_after_a_profile = |dir: Option<&Path>| {
            dir.and_then(Path::file_name)
                .and_then(|n| n.to_str())
                .is_some_and(|n| PROFILE_DIRS.contains(&n))
        };
        let dir = exe.parent();
        if named_after_a_profile(dir) || named_after_a_profile(dir.and_then(Path::parent)) {
            return Origin::Build;
        }
        // `skip(1)` — the ancestors of the FILE, so a binary that is itself called `target` is not
        // one of ours.
        if exe.ancestors().skip(1).any(|d| named(Some(d), TARGET_DIR)) {
            return Origin::Build;
        }
        if exe.ancestors().skip(1).any(is_a_cargo_target_dir) {
            return Origin::Build;
        }
        Origin::Installed
    }

    /// [`Origin::of`] against THIS process — through
    /// [`running_program`](crate::program::running_program), which CANONICALISES, and that is the
    /// whole of why it does.
    ///
    /// Two paths reach here as a symlink into a build tree and would otherwise be read as installed:
    /// the alias a launchd service is `exec`ed through (`~/.nexusflow-dev/bin/nexus-flow-dev`), and
    /// every job that service spawns — which runs that same alias path with `argv[0]` forced to
    /// `nxs`, so the name it could have been recognised by is gone and only the file is left.
    ///
    /// A process that cannot name its own binary is [`Origin::Build`]: not knowing must leave the
    /// environment the vote it has always had, never silently move a caller that asked for a named
    /// instance onto another instance's registry.
    pub fn ambient() -> Origin {
        match crate::program::running_program() {
            Some(exe) => Origin::of(&exe),
            None => Origin::Build,
        }
    }
}

/// Is `dir` a Cargo target directory — by its own marker file rather than by its name?
///
/// The signature is checked, not merely the file name: `CACHEDIR.TAG` is a shared convention (an
/// editor cache, a browser profile) and only the first line says who wrote it. A file that cannot
/// be read is not one, which is the safe answer here — the caller has already decided `Installed`
/// on two other grounds and this can only overrule that on evidence.
fn is_a_cargo_target_dir(dir: &Path) -> bool {
    let Ok(bytes) = std::fs::read(dir.join(CACHEDIR_TAG)) else {
        return false;
    };
    bytes.starts_with(CACHEDIR_SIGNATURE)
}

/// One named nexus-flow background service: its label, its home directory and its alias, from one
/// value.
///
/// The production instance is [`Instance::production`] and everything about it is byte-identical to
/// what this repo installed before instances existed — that is what makes an upgrade a no-op for
/// somebody who already has a service running.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instance {
    /// What follows `nexus-flow-`. Empty for production, which is what makes every derivation
    /// below a plain concatenation rather than a branch on a special case.
    qualifier: String,
}

impl Instance {
    /// The one service a machine had before this ticket, unchanged.
    pub fn production() -> Instance {
        Instance {
            qualifier: String::new(),
        }
    }

    /// Parse a full instance NAME (`nexus-flow`, `nexus-flow-dev`).
    ///
    /// The name is the spelling a person writes in an `.envrc`, the file name of the alias and the
    /// tail of the launchd label — ONE spelling everywhere, so that what a user sets is what they
    /// then see in their background items.
    pub fn named(name: &str) -> Result<Instance> {
        if name == BASE_NAME {
            return Ok(Instance::production());
        }
        let qualifier = name
            .strip_prefix(BASE_NAME)
            .and_then(|rest| rest.strip_prefix('-'))
            .ok_or_else(|| {
                NxfError::validation(format!(
                    "`{name}` is not a nexus-flow service instance name: it must be `{BASE_NAME}` \
                     or `{BASE_NAME}-<qualifier>` (for example `{BASE_NAME}-dev`)"
                ))
            })?;
        if qualifier.is_empty() || qualifier.len() > MAX_QUALIFIER {
            return Err(NxfError::validation(format!(
                "`{name}` is not a usable nexus-flow service instance name: the part after \
                 `{BASE_NAME}-` must be between 1 and {MAX_QUALIFIER} characters"
            )));
        }
        // The name becomes a directory under `$HOME` and a launchd label. Anything outside this
        // alphabet is refused rather than sanitised: a service that silently kept its state
        // somewhere other than where it was told is the failure this whole module exists to end.
        let legal = qualifier.split('-').all(|seg| {
            !seg.is_empty()
                && seg
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        });
        if !legal {
            return Err(NxfError::validation(format!(
                "`{name}` is not a usable nexus-flow service instance name: the part after \
                 `{BASE_NAME}-` must be lowercase letters and digits in dash-separated segments \
                 (it names a directory under your home and a launchd label)"
            )));
        }
        Ok(Instance {
            qualifier: qualifier.to_string(),
        })
    }

    /// The inverse of [`home_dir`](Instance::home_dir): which instance owns a directory found
    /// beside this one under `$HOME`.
    ///
    /// It exists for the one question two instances raise that neither of them can answer alone —
    /// "is this workspace also attended by somebody else?" — which means walking `$HOME` and
    /// deciding, per directory, whether it is one of ours. An unrelated dotfile directory is an
    /// error here, not a guess.
    pub fn from_home_dir(dir_name: &str) -> Result<Instance> {
        if dir_name == BASE_HOME_DIR {
            return Ok(Instance::production());
        }
        let qualifier = dir_name
            .strip_prefix(BASE_HOME_DIR)
            .and_then(|rest| rest.strip_prefix('-'))
            .ok_or_else(|| {
                NxfError::validation(format!(
                    "`{dir_name}` is not a nexus-flow service home directory"
                ))
            })?;
        Instance::named(&format!("{BASE_NAME}-{qualifier}"))
    }

    /// The instance this process belongs to, from the three inputs the module doc describes.
    ///
    /// `argv0` is the program name as invoked — the caller passes the BASENAME; a name that is not
    /// an instance name (`nxs`, `nxf`, a test binary) falls through to the rest rather than
    /// failing, because the overwhelming majority of invocations are not aliases.
    ///
    /// `origin` is where the running binary sits, and for [`Origin::Installed`] the environment is
    /// not consulted AT ALL (nxf 6j6v.cvpy) — not even to be validated. There is nothing for a
    /// malformed value to be loud about once it has no vote, and an installed suite that refused to
    /// run because a working copy exports a typo would be a new failure in place of the old one.
    ///
    /// For [`Origin::Build`], a `NXS_SERVICE_INSTANCE` that is set and malformed is an ERROR, never
    /// a quiet fall-back to production: somebody who asked for a named instance and got the shared
    /// one would be writing into everybody else's registry while believing they were isolated.
    ///
    /// A build with NO variable set is production, which is the same answer it has always given: a
    /// `cargo run` in a checkout that names no instance is not thereby a new service, it is the
    /// developer's own machine.
    pub fn resolve_from(
        argv0: Option<&str>,
        env: Option<&str>,
        origin: Origin,
    ) -> Result<Instance> {
        if let Some(from_alias) = argv0.and_then(|name| Instance::named(name).ok()) {
            return Ok(from_alias);
        }
        if origin == Origin::Installed {
            return Ok(Instance::production());
        }
        match env.map(str::trim) {
            None | Some("") => Ok(Instance::production()),
            Some(name) => Instance::named(name),
        }
    }

    /// [`resolve_from`](Instance::resolve_from) against this process: its own `argv[0]` basename,
    /// its own environment and its own binary's [`Origin`].
    pub fn ambient() -> Result<Instance> {
        let argv0 = std::env::args_os().next();
        let basename = argv0
            .as_deref()
            .and_then(|raw| std::path::Path::new(raw).file_name())
            .and_then(|n| n.to_str())
            .map(|n| n.strip_suffix(".exe").unwrap_or(n).to_string());
        Instance::resolve_from(
            basename.as_deref(),
            std::env::var(INSTANCE_ENV).ok().as_deref(),
            Origin::ambient(),
        )
    }

    /// `nexus-flow` / `nexus-flow-dev` — the alias's file name, the tail of the label, and the
    /// spelling every message about this instance uses.
    pub fn name(&self) -> String {
        if self.qualifier.is_empty() {
            BASE_NAME.to_string()
        } else {
            format!("{BASE_NAME}-{}", self.qualifier)
        }
    }

    /// `com.nxsflow.nexus-flow` — the launchd label, the plist's file stem, and the
    /// `bootstrap`/`bootout` target.
    pub fn label(&self) -> String {
        format!("{LABEL_PREFIX}{}", self.name())
    }

    /// `.nexusflow` / `.nexusflow-dev` — the directory name under `$HOME` that holds this
    /// instance's registry, lock, heartbeat, logs and alias.
    ///
    /// Separate homes are what make two instances structurally unable to fight: the single-instance
    /// lock is a file IN here, so two instances cannot share one without sharing this.
    pub fn home_dir(&self) -> String {
        if self.qualifier.is_empty() {
            BASE_HOME_DIR.to_string()
        } else {
            format!("{BASE_HOME_DIR}-{}", self.qualifier)
        }
    }

    /// Whether this is the one service a machine had before instances existed.
    ///
    /// The reason it is asked anywhere: only production has PREDECESSORS. The retired labels this
    /// repo boots out on install were all installed by the single unnamed service, so a
    /// sister instance must never sweep them — see [`crate::launchd::RETIRED_LABELS`].
    pub fn is_production(&self) -> bool {
        self.qualifier.is_empty()
    }

    /// What distinguishes a development instance — `dev` for `nexus-flow-dev` — or `None` for the
    /// production one. What a person reads beside a machine's name to tell two services on one
    /// computer apart (nxf 6j6v.f0b5).
    pub fn qualifier(&self) -> Option<&str> {
        (!self.qualifier.is_empty()).then_some(self.qualifier.as_str())
    }
}

/// **Say so when a command acted on a service the environment did not name** (nxf 6j6v.cvpy).
///
/// The state it describes is the one that produced the ticket: a working copy's `.envrc` exports
/// `NXS_SERVICE_INSTANCE=nexus-flow-foundations-dev`, and the `nxs` on the `PATH` is the INSTALLED
/// file — so a command that reads as "do this to the development service" does it to the production
/// one. It now does the right thing; without this sentence it would do it silently, and the person
/// would go on believing they had a development service.
///
/// **It lives here, and it is one sentence, because it has several callers** — the same argument as
/// [`crate::shared_workspace_note`]. `nxs sync daemon install`/`uninstall` act on a service; `nxs
/// sync bind`, `nxs sync unregister` and every `init` path change what a service ATTENDS. All of
/// them owe the reader the same sentence, and a sentence spelled five times is five sentences that
/// can drift (review of PR #441, Code Quality #2 / Integrity #2, which found it on two of the five).
///
/// `None` for the ordinary case: the variable unset, the variable naming production, or a build
/// binary — which is the state where it was obeyed. In a test that is always the answer, because a
/// test binary is itself a build, so nothing here can make an existing suite's output depend on the
/// environment its runner was started in.
///
/// The second half is built from the asked-for name rather than around a placeholder, so the alias
/// it offers is a path the reader can paste. A value that is not even a legal instance name is told
/// so HERE and nowhere else: for an installed binary the variable is never parsed, so this is the
/// only place a typo in it can still be named without failing a command that had no reason to fail.
pub fn instance_env_ignored_note(env: Option<&str>, origin: Origin) -> Option<String> {
    if origin != Origin::Installed {
        return None;
    }
    let asked = env.map(str::trim).filter(|n| !n.is_empty())?;
    let production = Instance::production();
    if asked == production.name() {
        return None;
    }
    let reach = match Instance::named(asked) {
        Ok(instance) => format!(
            "An instance that is already installed can also be addressed by its own alias, \
             `~/{}/bin/{} <verb>` — the same name the running service reads itself from, and it \
             stands for `nxs sync daemon`, so the verb follows it directly.",
            instance.home_dir(),
            instance.name()
        ),
        Err(e) => format!("It is not a usable instance name either: {}", e.msg),
    };
    Some(format!(
        "{INSTANCE_ENV}={asked} had no say here: this is the installed binary, and an installed \
         binary is always the {} service — so that is the one this command acted on. A development \
         instance belongs to a BUILD, so run the build itself (`./target/debug/nxs …`), which is \
         also what points that instance's alias at the binary you built. {reach}",
        production.name()
    ))
}

/// [`instance_env_ignored_note`] against THIS process — the environment it was started with and the
/// binary it is running. The form every caller uses; the pure one above is what a test drives.
pub fn ignored_instance_env() -> Option<String> {
    instance_env_ignored_note(
        std::env::var(INSTANCE_ENV).ok().as_deref(),
        Origin::ambient(),
    )
}

impl std::fmt::Display for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name())
    }
}

impl Default for Instance {
    fn default() -> Self {
        Instance::production()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_is_spelled_exactly_as_it_was_before_instances_existed() {
        // The upgrade-safety property, and it is the one worth pinning hardest: a machine with a
        // service already installed must find the same label, the same directory and the same
        // alias after this change, or the upgrade strands it.
        let p = Instance::production();
        assert_eq!(p.name(), "nexus-flow");
        assert_eq!(p.label(), "com.nxsflow.nexus-flow");
        assert_eq!(p.home_dir(), ".nexusflow");
        assert!(p.is_production());
    }

    #[test]
    fn a_named_instance_carries_its_name_into_all_three_derivations() {
        let dev = Instance::named("nexus-flow-dev").expect("a legal name");
        assert_eq!(dev.name(), "nexus-flow-dev");
        assert_eq!(dev.label(), "com.nxsflow.nexus-flow-dev");
        assert_eq!(dev.home_dir(), ".nexusflow-dev");
        assert!(!dev.is_production());
    }

    #[test]
    fn a_multi_segment_qualifier_is_legal_because_the_owners_own_names_use_one() {
        // `nexus-flow-foundations-dev` and `nexus-flow-manufakt-dev` are the two sister repos'
        // names from the ticket itself.
        for (name, home) in [
            ("nexus-flow-foundations-dev", ".nexusflow-foundations-dev"),
            ("nexus-flow-manufakt-dev", ".nexusflow-manufakt-dev"),
        ] {
            let i = Instance::named(name).expect("the owner's own names are legal");
            assert_eq!(i.home_dir(), home);
            assert_eq!(i.label(), format!("com.nxsflow.{name}"));
        }
    }

    #[test]
    fn no_two_instances_share_a_label_or_a_home_directory() {
        // The property the whole ticket rests on, asserted rather than assumed: separate homes are
        // what make separate locks, registries and heartbeats structural instead of hoped for.
        let names = [
            "nexus-flow",
            "nexus-flow-dev",
            "nexus-flow-foundations-dev",
            "nexus-flow-manufakt-dev",
        ];
        let instances: Vec<Instance> = names.iter().map(|n| Instance::named(n).unwrap()).collect();
        for (i, a) in instances.iter().enumerate() {
            for b in &instances[i + 1..] {
                assert_ne!(a.label(), b.label());
                assert_ne!(a.home_dir(), b.home_dir());
                assert_ne!(a.name(), b.name());
            }
        }
    }

    #[test]
    fn a_home_directory_names_the_instance_that_owns_it_and_nothing_else_does() {
        for name in ["nexus-flow", "nexus-flow-dev", "nexus-flow-foundations-dev"] {
            let i = Instance::named(name).unwrap();
            assert_eq!(
                Instance::from_home_dir(&i.home_dir()).unwrap(),
                i,
                "{name} does not survive the round trip through its own directory name"
            );
        }
        for stranger in [
            ".config",
            ".nexusflow.bak",
            ".nexusflowdev",
            "nexusflow",
            ".nexusflow-",
        ] {
            assert!(
                Instance::from_home_dir(stranger).is_err(),
                "`{stranger}` must not be read as one of ours"
            );
        }
    }

    #[test]
    fn a_name_that_could_reach_out_of_the_home_directory_is_refused_and_not_sanitised() {
        // Step 2 of the precedence takes this from an environment variable, so it is attacker- and
        // typo-reachable in a way a compiled constant never was.
        for hostile in [
            "nexus-flow-../evil",
            "nexus-flow-a/b",
            "nexus-flow-..",
            "nexus-flow-",
            "nexus-flow-dev ",
            "nexus-flow-DEV",
            "nexus-flow-a--b",
            "nexus-flow-dev-",
            "nexus-flow.dev",
            "nexusflow-dev",
            "nxs",
            "",
        ] {
            let err = Instance::named(hostile)
                .expect_err("`{hostile}` must not be accepted as an instance name");
            assert_eq!(err.kind.as_str(), "validation", "{hostile}");
        }
    }

    #[test]
    fn a_qualifier_past_the_ceiling_is_refused() {
        let long = format!("nexus-flow-{}", "a".repeat(MAX_QUALIFIER + 1));
        assert!(Instance::named(&long).is_err());
        let just_fits = format!("nexus-flow-{}", "a".repeat(MAX_QUALIFIER));
        assert!(Instance::named(&just_fits).is_ok());
    }

    #[test]
    fn the_running_service_learns_its_instance_from_the_name_it_was_started_as() {
        // The hard constraint from the ticket: `ProgramArguments` is one element, so `argv[0]` is
        // the ONLY channel a launchd-started service has.
        for origin in [Origin::Installed, Origin::Build] {
            assert_eq!(
                Instance::resolve_from(Some("nexus-flow-dev"), None, origin).unwrap(),
                Instance::named("nexus-flow-dev").unwrap(),
                "{origin:?}"
            );
            assert_eq!(
                Instance::resolve_from(Some("nexus-flow"), None, origin).unwrap(),
                Instance::production(),
                "{origin:?}"
            );
        }
    }

    #[test]
    fn the_alias_it_was_started_as_beats_whatever_the_environment_says() {
        // A `nexus-flow-dev` service started by launchd out of a login session whose environment
        // still names production must be the dev one — it is holding the dev lock and writing the
        // dev heartbeat, and reading anything else would make it report on a home it is not using.
        assert_eq!(
            Instance::resolve_from(Some("nexus-flow-dev"), Some("nexus-flow"), Origin::Build)
                .unwrap(),
            Instance::named("nexus-flow-dev").unwrap()
        );
    }

    #[test]
    fn a_build_that_is_not_an_alias_is_told_by_the_environment_which_working_copy_it_is() {
        for persona in ["nxs", "nxf", "nxm", "nxc", "service-9f3a21b0"] {
            assert_eq!(
                Instance::resolve_from(Some(persona), Some("nexus-flow-dev"), Origin::Build)
                    .unwrap(),
                Instance::named("nexus-flow-dev").unwrap(),
                "{persona}"
            );
        }
    }

    #[test]
    fn the_installed_binary_is_the_machines_own_service_whatever_the_working_copy_exports() {
        // The defect this rule replaces, at the seam that decided it (nxf 6j6v.cvpy): the `nxf` on
        // the `PATH` is the installed file, and standing in a checkout does not make it somebody
        // else's.
        for persona in ["nxs", "nxf", "nxm", "nxc"] {
            assert_eq!(
                Instance::resolve_from(
                    Some(persona),
                    Some("nexus-flow-foundations-dev"),
                    Origin::Installed
                )
                .unwrap(),
                Instance::production(),
                "{persona}"
            );
        }
    }

    #[test]
    fn nothing_set_anywhere_is_the_service_the_machine_already_had() {
        assert_eq!(
            Instance::resolve_from(None, None, Origin::Installed).unwrap(),
            Instance::production()
        );
        assert_eq!(
            Instance::resolve_from(Some("nxs"), Some("   "), Origin::Build).unwrap(),
            Instance::production(),
            "an exported-but-empty variable is `unset`, which is what a shell leaves behind"
        );
        assert_eq!(
            Instance::resolve_from(Some("nxs"), None, Origin::Build).unwrap(),
            Instance::production(),
            "a build that names no instance is the developer's own machine, not a new service"
        );
    }

    #[test]
    fn a_malformed_environment_variable_is_loud_where_it_has_a_vote_and_unread_where_it_has_none() {
        // Loud for a build: somebody who asked for isolation and got the shared registry would be
        // writing into everybody else's while believing they were alone.
        let err = Instance::resolve_from(Some("nxs"), Some("dev"), Origin::Build).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert!(err.msg.contains("nexus-flow"), "{}", err.msg);
        // And unread for an installed one: it has no vote, so there is nothing for it to be loud
        // about — an installed suite must not stop working because a checkout exports a typo.
        assert_eq!(
            Instance::resolve_from(Some("nxs"), Some("dev"), Origin::Installed).unwrap(),
            Instance::production()
        );
    }

    #[test]
    fn a_built_binary_and_every_test_binary_beside_it_is_a_build() {
        for path in [
            "/Users/u/Development/nexus-flow/target/debug/nxs",
            "/Users/u/Development/nexus-flow/target/release/nxs",
            // Every integration test binary Cargo builds, one level below the profile dir.
            "/Users/u/Development/nexus-flow/target/debug/deps/instance_isolation-9f3a21b0",
            "/Users/u/Development/nexus-flow/target/release/deps/e2e-1b2c3d4e",
            // `--target <triple>` and a redirected `CARGO_TARGET_DIR` both keep the profile dir,
            // which is why that — and not the word `target` — is what is read.
            "/Users/u/Development/nexus-flow/target/aarch64-apple-darwin/debug/nxs",
            "/Users/u/.cache/cargo-target/debug/nxs",
        ] {
            assert_eq!(Origin::of(Path::new(path)), Origin::Build, "{path}");
        }
    }

    #[test]
    fn a_profile_of_ones_own_is_a_build_too_because_the_target_directory_says_so() {
        // Review of PR #441 (Code Quality #1 / Integrity #1). The first cut asked only for the two
        // built-in profile names, so `cargo build --profile release-lto` read as INSTALLED — the
        // one direction that must not be wrong, because it moves a development build onto the
        // production registry in silence.
        for path in [
            "/Users/u/Development/nexus-flow/target/release-lto/nxs",
            "/Users/u/Development/nexus-flow/target/release-lto/deps/e2e-1b2c3d4e",
            "/Users/u/Development/nexus-flow/target/aarch64-apple-darwin/bench/nxs",
        ] {
            assert_eq!(Origin::of(Path::new(path)), Origin::Build, "{path}");
        }
    }

    #[test]
    fn everything_else_is_the_installed_suite() {
        for path in [
            // Where `install.sh` puts it, and the one that made this ticket.
            "/Users/u/.local/bin/nxs",
            "/Users/u/.local/bin/nxf",
            // `cargo install` — built, but installed, and the rule is about where it SITS.
            "/Users/u/.cargo/bin/nxs",
            "/usr/local/bin/nxs",
            "/opt/homebrew/bin/nxs",
            // A binary CALLED target is not a binary IN one.
            "/Users/u/.local/bin/target",
            "nxs",
        ] {
            assert_eq!(Origin::of(Path::new(path)), Origin::Installed, "{path}");
        }
    }

    #[test]
    fn a_build_tree_no_name_gives_away_is_found_by_cargos_own_marker() {
        // The third question, and the only one that survives the other two being wrong at once: a
        // private profile inside a redirected `CARGO_TARGET_DIR`, where neither a profile name nor
        // the word `target` appears anywhere in the path.
        let tmp = tempfile::TempDir::new().unwrap();
        let out = tmp.path().join("cache").join("shipping");
        std::fs::create_dir_all(&out).unwrap();
        let exe = out.join("nxs");
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();
        assert_eq!(
            Origin::of(&exe),
            Origin::Installed,
            "nothing in this path is a name we know, so the first two questions must say installed"
        );

        std::fs::write(
            tmp.path().join("cache").join(CACHEDIR_TAG),
            format!(
                "{}\n# written by cargo\n",
                String::from_utf8_lossy(CACHEDIR_SIGNATURE)
            ),
        )
        .unwrap();
        assert_eq!(
            Origin::of(&exe),
            Origin::Build,
            "and Cargo's own marker above it must overrule them"
        );
    }

    #[test]
    fn a_cache_tag_somebody_else_wrote_is_not_a_cargo_build_tree() {
        // `CACHEDIR.TAG` is a shared convention — an editor cache, a browser profile. Only the
        // signature says who wrote it, and reading the name alone would classify somebody's cache
        // directory as a build tree.
        let tmp = tempfile::TempDir::new().unwrap();
        let out = tmp.path().join("cache").join("bin");
        std::fs::create_dir_all(&out).unwrap();
        let exe = out.join("nxs");
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();
        std::fs::write(
            tmp.path().join("cache").join(CACHEDIR_TAG),
            "Signature: 0000000000000000000000000000000\n",
        )
        .unwrap();
        assert_eq!(Origin::of(&exe), Origin::Installed);
    }

    #[test]
    fn an_alias_path_and_its_target_are_two_different_verdicts_which_is_why_ambient_resolves() {
        // NOT a test of `Origin::ambient` — it cannot be, from in-process, with no control over
        // this test binary's own path (review of PR #441, Test Quality #1, which found this test
        // claiming more than it did). What it pins is the PREMISE that makes `ambient`'s
        // canonicalisation load-bearing: the two paths genuinely disagree, so which one is asked
        // decides the answer. That `ambient` asks the resolved one is proved where it can be —
        // against a real symlink and a real process — by
        // `crates/nxs/tests/the_running_binary_decides_the_instance.rs`'s
        // `a_job_started_through_a_development_alias_belongs_to_that_instance…`.
        let alias = Path::new("/Users/u/.nexusflow-dev/bin/nexus-flow-dev");
        assert_eq!(
            Origin::of(alias),
            Origin::Installed,
            "the link itself is not in a build tree"
        );
        assert_eq!(
            Origin::of(Path::new(
                "/Users/u/Development/nexus-flow/target/debug/nxs"
            )),
            Origin::Build,
            "and that is what the link resolves to when the instance was installed from a build"
        );
    }

    #[test]
    fn a_command_that_gave_the_variable_no_vote_says_which_service_it_acted_on() {
        // The exact state that produced nxf 6j6v.cvpy: a checkout exporting a development name,
        // and an installed binary doing what the person read as "…the development service".
        let note = instance_env_ignored_note(Some("nexus-flow-foundations-dev"), Origin::Installed)
            .expect("a named instance an installed binary cannot be is worth a sentence");
        assert!(note.contains("nexus-flow-foundations-dev"), "{note}");
        assert!(
            note.contains("nexus-flow service"),
            "it must name the service that was actually acted on: {note}"
        );
        assert!(
            note.contains("./target/debug/nxs"),
            "and the way to reach a development instance: {note}"
        );
        assert!(
            note.contains("~/.nexusflow-foundations-dev/bin/nexus-flow-foundations-dev <verb>"),
            "the alias it offers is a path, not a placeholder — and the verb follows the alias \
             directly, because the alias already stands for `nxs sync daemon`: {note}"
        );
    }

    #[test]
    fn nothing_is_said_when_the_variable_was_obeyed_absent_or_production() {
        // Silence everywhere it was not overruled — otherwise the note fires on every command this
        // repository's own `.envrc` is standing in, and a note that fires always is read never.
        assert_eq!(
            instance_env_ignored_note(Some("nexus-flow-dev"), Origin::Build),
            None,
            "a build obeys it, so there is nothing to report"
        );
        assert_eq!(instance_env_ignored_note(None, Origin::Installed), None);
        assert_eq!(
            instance_env_ignored_note(Some("  "), Origin::Installed),
            None,
            "an exported-but-empty variable is `unset`"
        );
        assert_eq!(
            instance_env_ignored_note(Some("nexus-flow"), Origin::Installed),
            None,
            "naming production and getting production is not an override"
        );
    }

    #[test]
    fn a_value_that_is_not_even_an_instance_name_is_said_to_be_one() {
        // The one place a typo can still be named. An installed binary never parses the variable —
        // that is the rule — so without this the value would be dropped in total silence.
        let note = instance_env_ignored_note(Some("dev"), Origin::Installed)
            .expect("a malformed value is still a value that was not used");
        assert!(note.contains("not a usable instance name"), "{note}");
        assert!(
            note.contains("nexus-flow-<qualifier>"),
            "and it carries the validator's own words about what a name may be: {note}"
        );
    }
}
