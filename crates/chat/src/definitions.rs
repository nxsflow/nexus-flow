//! The declaration catalogue both seams resolve roles and channels through (nxf 6j6v.h2fr, spec
//! `docs/specs/E5c-chat-orchestration-api.md` §3.2).
//!
//! Until nxf 6j6v.h2fr there was no catalogue at all: `cli.rs` re-derived the directory mid-verb
//! and called [`crate::role::load_all_roles`] / [`crate::channel::load_all_channels`] over and
//! over. That is fine for a process-per-invocation CLI and unworkable for an embedding app, which
//! held no catalogue it could resolve twice against.
//!
//! It carried the declared WORKFLOWS as a third kind until 6j6v.dvyq §3 removed the run engine.
//!
//! A [`Definitions`] is that catalogue as a value, and it has **ONE source: the folder.** It is
//! loaded by [`Definitions::resolve`] (a workspace root, resolving `.nxs-personas/` against a
//! legacy `roles/`) through [`Definitions::from_dir`] (one named folder), and both build through
//! [`Definitions::new`] — the validating constructor, which is a CONSTRUCTION SEAM and not a second
//! source. Every lookup a verb needs is a method on the result.
//!
//! **h2fr's answer to the app was different, and 6j6v.dvyq step 6 reversed it.** This header used
//! to say a catalogue was "loaded from a folder OR handed over by the caller", because an app was
//! not supposed to have to write a definition folder into a user's project. It does now, and that
//! is the point: personas and channels live in `.nxs-personas/` for an app exactly as for the CLI,
//! even a later in-app role editor writes there, and `DefinitionSource::Supplied` /
//! `Engine::set_definitions` are gone. `tests/definitions.rs` carries the same account.
//!
//! Since 6j6v.dvyq the folder is `<workspace-root>/.nxs-personas/`, and WHICH folder a catalogue
//! came from is itself part of the answer — see [`DeclarationSource`].
//!
//! **Since 6j6v.7k58 there is a second folder, and the two are MERGED PER NAME.** Beside the
//! workspace's own folder every workspace reads the USER-LEVEL folder
//! ([`user_declarations_dir`], `~/.nexusflow/personas/` for the installed suite): a persona or a
//! channel the workspace declares itself hides the user-level one of the same name, everything else
//! is added. One definition `pm` therefore runs in every repository as that repository's own
//! participant — its board, its memory, its working copy — while it is written down once. The price
//! is said where it is paid: a declaration outside the repository is not versioned with it, and a
//! clone on a machine without the folder has no `pm`. That is why the catalogue says, per entry,
//! where it came from ([`DeclarationSource::from_user`]) and which user-level entries the workspace
//! hides ([`DeclarationSource::shadowed`]).
//!
//! The merge happens HERE, inside [`Definitions::resolve`], and nowhere else — so every seam that
//! resolves a catalogue (the CLI per invocation, every `Engine` verb, `nxs prime`, a spawned
//! persona's own `nxc`) sees the same team, and everything built on the catalogue inherits the
//! user-level entries without knowing they exist: the per-operation freeze
//! ([`crate::declaration_freeze`]) projects the MERGED catalogue, so a hurdle (`preconditions:`) that
//! arrives from the user-level folder is frozen at an operation's start exactly like one from the
//! workspace, and an edit to the user-level folder reaches the next operation, not a running one.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::channel::{ChannelDecl, RESERVED_HANDLE_PREFIX};
use crate::error::{NxfError, Result};
use crate::role::RoleDecl;
use crate::workspace::{LEGACY_ROLES_DIR, PERSONAS_DIR};

/// Which folder a workspace's declarations came from — the FIRST-CLASS resolution result
/// (nxf 6j6v.dvyq, note of 2026-08-14, answering app-foundations).
///
/// It exists because the empty case had to stop being anonymous. An empty catalogue is
/// indistinguishable from "there is nothing here", and app-side that becomes a blank window with no
/// explanation — the same failure 6j6v.hpv8 already fixed for the trigger path. So the resolution
/// travels WITH the catalogue: [`Definitions::source`] carries it, and every surface renders the
/// same fact in its own register (`list`/`prime` say it and exit 0; `send --to` refuses, because an
/// action with a mandatory target has none; [`crate::engine::Engine::definitions`] simply returns
/// an empty catalogue, because a read that fails BECAUSE there is nothing to read is a bad read).
///
/// **One structure answers both of app-foundations' questions**, which is why the legacy folder is
/// reported here and not through a second channel: "where does this catalogue come from" and "was a
/// legacy `roles/` folder found and read" are the same question asked twice.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeclarationSource {
    /// Where a declaration BELONGS in this workspace: `<workspace-root>/.nxs-personas`. Always the
    /// new location, whatever was actually read — this is the path a surface prints when it has to
    /// say "there is nothing declared, and here is where you put it".
    pub path: PathBuf,
    /// Which of the two locations the catalogue in hand was read from.
    pub kind: DeclarationSourceKind,
    /// The legacy `<workspace-root>/roles` folder, when one was FOUND in this workspace — the
    /// machine-readable migration report the 2026-08-14 decision demands. Present whether or not it
    /// won the resolution, because an app offering "move this folder" needs to know it is there
    /// even when `.nxs-personas/` already carries the declarations.
    ///
    /// Omitted from the wire form when absent, rather than serialized as `null`: a workspace that
    /// never had one keeps the smaller shape, and a reader branches on presence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_path: Option<PathBuf>,
    /// The folder that was actually read. Equal to [`path`](DeclarationSource::path) when nothing
    /// is declared at all — every loader reads a missing folder as an empty catalogue, so the
    /// resolution stays total.
    pub read_dir: PathBuf,
    /// How many declarations the catalogue carries: personas + channels — the workspace's own and
    /// the user-level ones it does not hide, together.
    pub count: usize,
    /// The USER-LEVEL folder (nxf 6j6v.7k58, see [`user_declarations_dir`]) — present only when it
    /// carries at least one declaration, so a workspace on a machine without one keeps exactly the
    /// shape (and the wording) it had before the folder existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_path: Option<PathBuf>,
    /// The entries of the catalogue that came from [`user_path`](DeclarationSource::user_path)
    /// rather than from the workspace's own folder — the per-entry origin `nxc list` and
    /// [`crate::engine::Engine::directory`] stamp onto the directory.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub from_user: Vec<DeclaredName>,
    /// User-level entries the workspace HIDES with its own declaration of the same name. Reported
    /// rather than resolved silently: a repository that keeps an old copy of a persona the owner
    /// now maintains centrally runs the old copy, and nobody would know.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shadowed: Vec<DeclaredName>,
    /// The file behind every entry of the merged catalogue (nxf 6j6v.dw16), in load order. Not on
    /// this record's wire form — it is projected onto each directory entry instead, where a reader
    /// looks for one persona's source.
    #[serde(skip)]
    pub files: Vec<DeclarationFile>,
}

/// One declaration by kind and name — how [`DeclarationSource`] names an entry's origin and a
/// shadowing (nxf 6j6v.7k58).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeclaredName {
    pub kind: DeclarationKind,
    pub name: String,
}

/// The two kinds of declaration that merge per name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclarationKind {
    Persona,
    Channel,
}

impl DeclarationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeclarationKind::Persona => "persona",
            DeclarationKind::Channel => "channel",
        }
    }
}

/// Where ONE entry of a catalogue came from (nxf 6j6v.7k58) — carried on the directory's entries,
/// and absent from their wire form for the common case, so a workspace without a user-level folder
/// renders exactly as before.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclarationOrigin {
    /// The workspace's own folder.
    #[default]
    Workspace,
    /// The user-level folder.
    User,
}

impl DeclarationOrigin {
    pub fn is_workspace(&self) -> bool {
        *self == DeclarationOrigin::Workspace
    }
}

/// The form a declaration was written in (nxf 6j6v.dw16 / 6j6v.k3qy) — said in `--json` only; to a
/// human a persona is a persona.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclarationForm {
    /// A persona as `<handle>.yaml` — the older form, read until it is migrated.
    Yaml,
    /// A persona as `<name>/SKILL.md` ([`crate::skill`]).
    Skill,
    /// A channel in the list of `channels.yaml` — the older form, read until it is migrated.
    ChannelList,
    /// A channel as `channels/<name>.yaml`, one per file.
    ChannelFile,
}

/// The FILE one entry of a catalogue was read from, and what that file says beyond the
/// declaration every seam resolves through (nxf 6j6v.dw16): the fields a skill passes through, the
/// capabilities it names, and the keys that were read and ignored.
///
/// Carried on [`DeclarationSource::files`] for every entry of the merged catalogue, and stamped
/// onto the directory's entries, where `--json` shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeclarationFile {
    pub kind: DeclarationKind,
    pub name: String,
    pub file: PathBuf,
    pub form: DeclarationForm,
    /// The folder a relative reference in the declaration means: a skill's OWN folder, which is
    /// the Agent Skills rule; for the other forms the declaration folder that holds the file —
    /// shown for completeness, while a YAML persona's relative paths keep meaning the repository
    /// the session runs in.
    pub folder: PathBuf,
    /// `nxs.requires` — capabilities the persona names. Read and shown, NOT checked and NOT bound in
    /// this release.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// Passed through from the Agent Skills specification: not read, preserved, shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Keys the file declares that were read and ignored — the material of the declaration
    /// warnings `nxs prime` shows, not part of the wire form.
    #[serde(skip)]
    pub ignored: Vec<Ignored>,
}

/// One key a declaration file carries that has no effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ignored {
    /// `address_book` — retired (nxf 6j6v.xjh3), with the number of entries it listed.
    AddressBook { entries: usize },
    /// `session`, `sub_agents` or `reports_to` — never had an effect.
    Inert(String),
    /// A nexus-flow key at the top level of a skill, where it is not read.
    Misplaced(String),
    /// A key under `nxs:` that names no field.
    Unknown(String),
}

/// The USER-LEVEL declaration folder this process reads beside every workspace's own (nxf
/// 6j6v.7k58): `<service home>/personas` — `~/.nexusflow/personas` for the installed suite and for
/// an embedding app, `~/.nexusflow-<qualifier>/personas` for a development build named so by
/// `NXS_SERVICE_INSTANCE` ([`nxs_service::ServiceHome::personas`]). `None` when there is none to
/// read — never an error.
///
/// **One rule, the service's, and no knob.** An embedding app does not choose this location, and
/// must not be able to: the personas it starts run the `nxc` CLI, which resolves the folder by this
/// same rule, and an app reading a folder of its own would disagree with the very sessions it
/// spawned about who exists.
///
/// **A build never reads the production folder** (review of PR #17, Code Quality #2 / Integrity
/// #3). The service rule sends a build that names no instance to the production home, which is
/// right for a registry nobody fills by hand and wrong here: a test binary is such a build, and
/// `cargo test` run outside `direnv` — CI, another shell — would read the owner's real personas
/// into every suite that resolves a catalogue. So a binary inside a build directory reads a
/// user-level folder only under a NAMED development instance (`~/.nexusflow-<qualifier>/personas`),
/// and the installed suite — the one thing that has a production folder to read — is untouched. A
/// suite that tests this folder pins the home and then names a development instance.
///
/// **The environment cannot fail a catalogue** (review of PR #17, Code Quality #3 / Integrity #4).
/// No home directory, or an instance name that does not validate, means no user-level folder — the
/// state of every machine before this folder existed — rather than an error on every read.
pub fn user_declarations_dir() -> Option<PathBuf> {
    let instance = nxs_service::Instance::ambient().ok()?;
    if instance.is_production() && nxs_service::Origin::ambient() == nxs_service::Origin::Build {
        return None;
    }
    nxs_service::ServiceHome::for_instance(instance)
        .ok()
        .map(|home| home.personas())
}

/// The declared CHANNELS of a workspace, merged with the user-level ones — the read for a verb that
/// needs only the channel policy (`threads show`, `search`, `Engine::thread`, `Engine::search`), so
/// that it depends on the two `channels.yaml` files and not on every persona file besides (review
/// of PR #17, Code Quality #1: a malformed persona file anywhere must not make a thread unreadable).
/// Same rule as [`Definitions::resolve`]: a workspace channel hides the user-level one of its name.
pub fn merged_channels(root: &Path) -> Result<Vec<ChannelDecl>> {
    merged_channels_with_user_dir(root, user_declarations_dir().as_deref())
}

/// [`merged_channels`] against a NAMED user-level folder, or none — the test form, as
/// [`Definitions::resolve_with_user_dir`] is for the whole catalogue.
pub fn merged_channels_with_user_dir(
    root: &Path,
    user_dir: Option<&Path>,
) -> Result<Vec<ChannelDecl>> {
    let source = DeclarationSource::locate(root);
    let mut channels = crate::channel::load_all_channels(&source.read_dir)?;
    if let Some(user_dir) = user_folder_carrying_declarations(user_dir)? {
        let user_channels = crate::channel::load_all_channels(&user_dir)?;
        for channel in user_channels {
            if !channels.iter().any(|c| c.name == channel.name) {
                channels.push(channel);
            }
        }
    }
    Ok(channels)
}

/// `dir` when it carries a declaration, `None` when it does not exist or carries none — and an
/// ERROR when it exists but cannot be read (review of PR #17, Integrity #5): an unreadable
/// user-level folder would otherwise make its personas vanish from every workspace without a word,
/// which is the fail-open the rest of this module refuses.
fn user_folder_carrying_declarations(dir: Option<&Path>) -> Result<Option<PathBuf>> {
    let Some(dir) = dir else {
        return Ok(None);
    };
    if !dir.exists() {
        return Ok(None);
    }
    std::fs::read_dir(dir).map_err(|e| {
        NxfError::io(format!(
            "reading the user-level declaration folder {}: {e}",
            dir.display()
        ))
    })?;
    Ok(carries_declarations(dir).then(|| dir.to_path_buf()))
}

/// The three answers to "where did this catalogue come from".
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclarationSourceKind {
    /// Read from `<workspace-root>/.nxs-personas/` — the location declarations belong in.
    Personas,
    /// Read from a LEGACY `<workspace-root>/roles/` folder. Read, reported, never rewritten.
    LegacyRoles,
    /// Neither folder carries a declaration. Not an error: a fresh workspace has no team yet.
    None,
}

impl DeclarationSourceKind {
    /// The wire/`--json` spelling, and what a human surface prints. Pinned equal to the derived
    /// `Serialize` form by `tests/declaration_source.rs`, so the two cannot drift apart.
    pub fn as_str(&self) -> &'static str {
        match self {
            DeclarationSourceKind::Personas => "personas",
            DeclarationSourceKind::LegacyRoles => "legacy_roles",
            DeclarationSourceKind::None => "none",
        }
    }
}

impl DeclarationSource {
    /// Resolve the two candidate folders against a workspace ROOT (the parent of `.nxs/`).
    ///
    /// **The engine reads BOTH locations and `.nxs-personas/` wins the tie** (decision of
    /// 2026-08-14, recorded on 6j6v.dvyq). "Wins the tie" is resolved here as *carries at least one
    /// declaration*, not merely *exists*, and that distinction is load-bearing: `nxc init` now
    /// creates an EMPTY `.nxs-personas/`, and an existence-only rule would let that empty folder
    /// shadow a legacy `roles/` folder full of declarations — silently unteaming a workspace by
    /// running `init` in it.
    ///
    /// **It does not merge the two.** One catalogue, from one folder, so a lookup has one answer
    /// and [`Definitions::new`]'s duplicate-name rule keeps meaning what it says. That is also the
    /// only reading under which [`DeclarationSourceKind`]'s three values can name the provenance at
    /// all.
    ///
    /// **And it never writes.** An application that has only OPENED someone else's project must not
    /// silently rewrite it: that is an irreversible change to a third party's property, triggered
    /// by merely looking. A tool an app or a human invokes explicitly performs the move; what lives
    /// here is a READING compatibility with a report, and removing that is a separate, later item.
    ///
    /// Pure filesystem inspection — no YAML is parsed, so a malformed file in the LOSING folder
    /// cannot fail a workspace whose declarations are fine.
    ///
    /// **Crate-internal, deliberately**: the [`count`](DeclarationSource::count) it returns is `0`
    /// because only loading can answer it, and a caller that took this for the whole resolution
    /// would report "nothing is declared" for a workspace that declares plenty. That is not
    /// hypothetical — `Engine::prime_as` did exactly that for one commit, and the parity
    /// differential caught it against `nxc prime`. [`DeclarationSource::resolve`] is the answer
    /// with the count in it, and is what every caller outside this module gets.
    pub(crate) fn locate(root: &Path) -> DeclarationSource {
        // The caller's OWN spelling of the root, not a canonicalized one. Considered and rejected:
        // canonicalizing would make two callers who reached the same workspace by different routes
        // report the same string, which is convenient for a differential — but this path is shown
        // to a HUMAN ("declare one as <path>/<name>/SKILL.md"), and a path that is not the one they
        // opened the project with is worse than one that varies. It also matches every other path
        // this crate reports, `Workspace.dir` included, so there is one rule rather than two.
        let path = root.join(PERSONAS_DIR);
        let legacy = root.join(LEGACY_ROLES_DIR);
        let legacy_path = legacy.is_dir().then(|| legacy.clone());
        let (kind, read_dir) = if carries_declarations(&path) {
            (DeclarationSourceKind::Personas, path.clone())
        } else if carries_declarations(&legacy) {
            (DeclarationSourceKind::LegacyRoles, legacy)
        } else {
            (DeclarationSourceKind::None, path.clone())
        };
        DeclarationSource {
            path,
            kind,
            legacy_path,
            read_dir,
            count: 0,
            user_path: None,
            from_user: Vec::new(),
            shadowed: Vec::new(),
            files: Vec::new(),
        }
    }

    /// [`locate`](DeclarationSource::locate) plus the COUNT, which only loading can answer. The
    /// resolution on its own; [`Definitions::resolve`] is the same work when you also want the
    /// catalogue.
    pub fn resolve(root: &Path) -> Result<DeclarationSource> {
        Ok(Definitions::resolve(root)?
            .source
            .expect("Definitions::resolve always records its source"))
    }

    /// The one-line explanation a surface prints when there is nothing to show: what is declared,
    /// where it belongs, and — when one is lying around — that a legacy folder was seen.
    ///
    /// This is `list`/`prime`'s "says so, with the path": ORIENTATION, exit 0. An empty list on its
    /// own is a lie by omission; a failure would be wrong too, since having declared nothing yet is
    /// a legitimate state.
    pub fn explain(&self) -> String {
        let repo = self.explain_workspace_half();
        match &self.user_path {
            None => repo,
            Some(user) => format!("{repo} {}", self.explain_user_half(user)),
        }
    }

    /// The workspace's own folder, said as it was before the user-level folder existed — so a
    /// machine without one reads exactly as it did (acceptance point 8 of 6j6v.70dy).
    fn explain_workspace_half(&self) -> String {
        let where_it_belongs = format!(
            "declare one as `{}/<name>/SKILL.md` (a persona) or `{}/channels/<name>.yaml` (a channel)",
            self.path.display(),
            self.path.display()
        );
        // The workspace's OWN count: `count` is the merged catalogue's.
        let count = self.count.saturating_sub(self.from_user.len());
        match (self.kind, &self.legacy_path) {
            // With user-level entries in the catalogue, "nobody is declared here" would be false:
            // somebody is, just not by this repository.
            (DeclarationSourceKind::None, _) if self.user_path.is_some() => {
                format!("this repository declares nobody of its own — {where_it_belongs}.")
            }
            (DeclarationSourceKind::None, Some(legacy)) => format!(
                "nobody is declared here yet — {where_it_belongs}. A legacy folder exists at {} \
                 but declares nothing.",
                legacy.display()
            ),
            (DeclarationSourceKind::None, None) => {
                format!("nobody is declared here yet — {where_it_belongs}.")
            }
            (DeclarationSourceKind::LegacyRoles, _) => format!(
                "read from the LEGACY folder {} ({} declared). Declarations belong in {} now; move \
                 the folder when it suits you — nothing here rewrites your project.",
                self.read_dir.display(),
                count,
                self.path.display()
            ),
            (DeclarationSourceKind::Personas, Some(legacy)) => format!(
                "read from {} ({} declared). A legacy folder is still present at {} and is NOT \
                 read while {} carries declarations.",
                self.path.display(),
                count,
                legacy.display(),
                PERSONAS_DIR
            ),
            (DeclarationSourceKind::Personas, None) => {
                format!("read from {} ({} declared).", self.path.display(), count)
            }
        }
    }

    /// The user-level folder: what came from it, and what this workspace hides of it.
    fn explain_user_half(&self, user: &Path) -> String {
        let names = |list: &[DeclaredName]| {
            list.iter()
                .map(|d| format!("{} `{}`", d.kind.as_str(), d.name))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut out = match self.from_user.is_empty() {
            true => format!(
                "The user-level folder {} adds nothing here.",
                user.display()
            ),
            false => format!(
                "From the user-level folder {}: {} — not versioned with this repository.",
                user.display(),
                names(&self.from_user)
            ),
        };
        if !self.shadowed.is_empty() {
            out.push_str(&format!(
                " This repository's own declaration HIDES the user-level {}.",
                names(&self.shadowed)
            ));
        }
        out
    }

    /// Whether a surface that shows a non-empty directory should print [`explain`] beside it: a
    /// LEGACY folder is being read, or the user-level folder contributes or is hidden. The plain
    /// case — the workspace's own `.nxs-personas/` and nothing else — has nothing to say.
    ///
    /// [`explain`]: DeclarationSource::explain
    pub fn worth_saying(&self) -> bool {
        self.kind == DeclarationSourceKind::LegacyRoles || self.user_path.is_some()
    }

    /// The file the entry `name` of `kind` in this catalogue was read from (nxf 6j6v.dw16).
    pub fn file_of(&self, kind: DeclarationKind, name: &str) -> Option<&DeclarationFile> {
        self.files.iter().find(|f| f.kind == kind && f.name == name)
    }

    /// Where the entry `name` of `kind` in this catalogue came from.
    pub fn origin_of(&self, kind: DeclarationKind, name: &str) -> DeclarationOrigin {
        match self
            .from_user
            .iter()
            .any(|d| d.kind == kind && d.name == name)
        {
            true => DeclarationOrigin::User,
            false => DeclarationOrigin::Workspace,
        }
    }

    /// The `--json` projection — the same fact the human sentence carries, as data.
    ///
    /// Delegates to the derived `Serialize` rather than assembling a second object by hand: two
    /// ways to serialize one record is exactly how two views drift apart (the same reasoning
    /// [`crate::facade::PrimeReport`] states for refusing to derive `Serialize` at all — there the
    /// single projection is hand-written, here it is derived; either way there is only one).
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("a declaration source serializes")
    }
}

/// The name of the explainer `nxc init` drops into a fresh `.nxs-personas/`.
pub const PERSONAS_README: &str = "README.md";

/// What that explainer says. A workspace ships with NO roles — shipped standard personas are
/// explicitly not part of this epic (owner note on 6j6v.m4xe) — so the folder arrives empty, and an
/// empty folder with no explanation is exactly the anonymous blank this item exists to abolish.
pub const PERSONAS_README_BODY: &str = "\
# .nxs-personas

Who your agents are, and where they talk. nexus-chat reads this folder — nothing writes it but you.

## One folder per persona: `<name>/SKILL.md`

```markdown
---
name: coder
description: Builds what the work order asks for, and says so when it cannot.
nxs:
  title: Implementer
---
You implement one task at a time and report back in the thread you were asked in.
```

A persona is a skill in the form of the Agent Skills specification: a skill folder you found
elsewhere works when you copy it in. Address it with `nxc send --to coder -`, with the request on
STDIN — `… - <<'EOF'`, your text, then `EOF` on its own line — so the shell evaluates nothing in
it. A one-liner may be an argument instead.

## One file per channel: `channels/<name>.yaml`

```yaml
name: review
members: [coder, reviewer]
timeout: 30m
```

Address it with `nxc send --to review -`, the same way. A channel may also declare `steps:` — its
members in order — and then a send walks that order step by step.

The older forms, `<handle>.yaml` and a `channels.yaml` list, are still read;
`nxs personas migrate` rewrites them. `nxc guide personas` has every field.

## Nothing is declared until it is in here

There are no built-in personas and no ad-hoc targets: `nxc send --to` reaches what this folder
declares, and nothing else. That is the point — a target that is declared is readable, reviewable,
and survives the run.
";

/// Whether a folder carries at least one declaration — a top-level `*.yaml`, a sub-folder with a
/// `SKILL.md`, or a `channels/*.yaml`. Cheap and parse-free
/// on purpose (see [`DeclarationSource::locate`]); an unreadable directory answers `false`, which
/// is the same answer the loaders give it.
///
/// It also looked into a `workflows/` subdirectory until 6j6v.dvyq §3 removed the run engine; a
/// folder holding nothing but those files declares no team and answers `false`, which is what the
/// loaders below now say too.
fn carries_declarations(dir: &Path) -> bool {
    fn has_yaml(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        entries
            .flatten()
            .any(|e| e.path().extension().is_some_and(|ext| ext == "yaml") && e.path().is_file())
    }
    // A skill folder (nxf 6j6v.dw16) or a channel file (nxf 6j6v.k3qy) is a declaration as much as
    // a YAML file beside them is — a folder holding only the new form must not read as empty.
    fn has_skill(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        entries
            .flatten()
            .any(|e| e.path().join(crate::skill::SKILL_FILE).is_file())
    }
    has_yaml(dir) || has_skill(dir) || has_yaml(&dir.join(crate::channel::CHANNELS_DIR))
}

/// Every role and channel declared for one workspace.
///
/// Validated at construction (see [`Definitions::new`]), so a verb resolving through it can trust
/// the shape of what it gets back and only has to handle "declared vs not declared".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Definitions {
    roles: Vec<RoleDecl>,
    channels: Vec<ChannelDecl>,
    /// Where this catalogue came from, when it was resolved against a workspace
    /// ([`Definitions::resolve`]). `None` for a catalogue a caller built in memory with
    /// [`Definitions::new`], which has no folder to have come from — every catalogue an embedding
    /// app receives from [`crate::engine::Engine::definitions`] carries `Some`.
    source: Option<DeclarationSource>,
}

impl Definitions {
    /// Build a catalogue from caller-supplied declarations, validating the structural guarantees
    /// the directory layout used to give implicitly:
    ///
    /// - **role handle form** — non-empty, no `/`, no `\`, no `..`. The path-traversal reason this
    ///   guard originally existed for (a handle feeding `roles.join(format!("{handle}.yaml"))`)
    ///   does not apply to host-supplied definitions, but a handle still becomes half of a
    ///   qualified `origin/handle` chat identity, and a `/` inside it makes that identity
    ///   unparseable back into its two halves. The rejection survives the move for a reason that
    ///   survives with it.
    /// - **reserved handles** — anything starting with [`RESERVED_HANDLE_PREFIX`] belongs to the
    ///   engine's own synthetic identities (`__synth__`, `__delivered__`, `__channel__`); a declared
    ///   role sharing one would collide with that mechanism once qualified into
    ///   `expects_reply_from` or compared against a thread's `opener`.
    /// - **uniqueness** — of role handles and channel names. One file per name made this
    ///   impossible before; a caller-supplied `Vec` does not, and a duplicate would mean a lookup
    ///   silently picks one of two declarations.
    /// - **a channel FLOW that reaches itself** ([`crate::channel::flow_cycles`], nxf 6j6v.hq71) —
    ///   see the paragraph after the next for why this one is here while referential integrity is
    ///   not.
    ///
    /// **Referential integrity is deliberately NOT checked here** (a channel naming a role that
    /// isn't declared). That stays advisory in `nxs prime` — which exists to REPORT a broken team
    /// roster rather than refuse to start — and fail-closed at the point of use via
    /// [`Definitions::declared_channel`]. Promoting it to a construction error would break prime's
    /// contract, and would make one bad channel reference stop an app from opening a workspace at
    /// all.
    ///
    /// **A flow cycle is on the other side of that line, and the difference is what it COSTS.** A
    /// dangling member reference degrades: the channel cannot fan out, and the one verb that tries
    /// says so. A cycle does not degrade — it runs, opening a real thread and starting a real paid
    /// session at every hop until `orchestration::MAX_HOP` stops the chain. Refusing a workspace over
    /// it is the cheaper of the two answers, and it is the only gate there is: `validate_channels` is
    /// advisory, and `cargo xtask channels check` never reads a declaration's content at all (it
    /// enforces the three release promotion rings).
    ///
    /// The counter-cost is real and is accepted rather than overlooked: this refuses the WHOLE
    /// catalogue, so one cyclic channel stops an app opening the workspace at all — the very outcome
    /// the paragraph above declines to impose for a dangling reference. It is the right trade only
    /// because the failure it prevents is unbounded and billable while the one it imposes is a
    /// startup error naming the two channels to edit.
    pub fn new(roles: Vec<RoleDecl>, channels: Vec<ChannelDecl>) -> Result<Definitions> {
        for role in &roles {
            validate_role_handle(&role.handle)?;
            // The folder `channels/` holds one file per channel (nxf 6j6v.k3qy), so the name is
            // the channels' — a persona called so would be one folder name for two kinds.
            if role.handle == crate::channel::CHANNELS_DIR {
                return Err(NxfError::validation(format!(
                    "a persona may not be called {:?}: the name is reserved for the folder that \
                     holds one file per channel",
                    role.handle
                )));
            }
        }
        reject_duplicates("role handle", roles.iter().map(|r| r.handle.as_str()))?;
        reject_duplicates("channel name", channels.iter().map(|c| c.name.as_str()))?;
        // **A flow that reaches itself is refused here, not merely reported** (nxf 6j6v.hq71). Since
        // a channel's flow step may address another CHANNEL, a catalogue can describe a cycle — and
        // a cycle is not a declaration that degrades, it is one that opens a fresh thread and starts
        // a fresh paid session at every hop until `orchestration::MAX_HOP` stops the chain.
        //
        // This is the GATE the work order looked for and did not find: `validate_channels` is
        // advisory (`nxc prime` prints it), `cargo xtask channels check` is about the three release
        // promotion rings and never reads a declaration's content, and there is no other. Every seam
        // — the CLI and every `Engine` — builds its catalogue through this constructor, so refusing
        // it here refuses it everywhere, exactly as the duplicate-name and role-handle rules above do.
        if let Some(err) = crate::channel::flow_cycles(&roles, &channels)
            .into_iter()
            .next()
        {
            return Err(NxfError::validation(format!("{}: {}", err.file, err.what)));
        }
        Ok(Definitions {
            roles,
            channels,
            source: None,
        })
    }

    // `with_prompt_workflow` and `prompt_workflow` stood here: which declared workflow's per-role
    // block `trigger_role` folded into a summoned role's system prompt. REMOVED with the run
    // engine (6j6v.dvyq §3). A persona is told what it is part of by the channel it was summoned
    // through and by the thread it must answer into.

    /// Load the catalogue from ONE declaration directory — the three existing loaders, then the
    /// same validation [`Definitions::new`] applies. A missing directory is an empty catalogue, not
    /// an error (each loader's own established contract): a fresh workspace with no declared team
    /// resolves cleanly.
    ///
    /// This reads the folder it is GIVEN and resolves nothing: use [`Definitions::resolve`] to get
    /// the folder a workspace's declarations actually live in, and with it the
    /// [`DeclarationSource`] every surface renders.
    pub fn from_dir(dir: &Path) -> Result<Definitions> {
        Definitions::new(
            crate::role::load_all_roles(dir)?,
            crate::channel::load_all_channels(dir)?,
        )
    }

    /// Resolve a WORKSPACE ROOT (the parent of `.nxs/`) into its catalogue, carrying the
    /// [`DeclarationSource`] it came from. This is what both seams call: `nxc` per invocation, and
    /// [`crate::engine::Engine`] on every verb that needs declarations.
    ///
    /// A workspace that declares nothing resolves to an EMPTY catalogue with a
    /// [`DeclarationSourceKind::None`] source — success, not an error. A read that fails because
    /// there is nothing to read is a bad read, and it would turn a legitimate state into an
    /// exception an app has to catch merely to render an onboarding screen (nxf 6j6v.dvyq, note of
    /// 2026-08-14; pinned on the app side by `engine-bridge/src/chat.rs` and
    /// `engine-server/tests/chat_wire.rs`).
    pub fn resolve(root: &Path) -> Result<Definitions> {
        Definitions::resolve_with_user_dir(root, user_declarations_dir().as_deref())
    }

    /// [`resolve`](Definitions::resolve) against a NAMED user-level folder, or none — the form for
    /// a test that must not read the machine's real one, and for a caller that has resolved the
    /// folder already. Production reads the one [`user_declarations_dir`] names; see there for why
    /// no seam lets an app pick another.
    pub fn resolve_with_user_dir(root: &Path, user_dir: Option<&Path>) -> Result<Definitions> {
        let mut source = DeclarationSource::locate(root);
        source.user_path = user_folder_carrying_declarations(user_dir)?;
        Definitions::load(source)
    }

    /// Re-read the catalogue a [`DeclarationSource`] describes — both folders, merged by the same
    /// rule. For a caller that was handed the resolution rather than the root (`prime`).
    pub fn from_source(source: &DeclarationSource) -> Result<Definitions> {
        let mut source = source.clone();
        source.from_user.clear();
        source.shadowed.clear();
        source.files.clear();
        Definitions::load(source)
    }

    /// Load the workspace's folder and, when there is one, the user-level folder, and MERGE them
    /// per name (nxf 6j6v.7k58): a workspace entry hides the user-level entry of the same name and
    /// kind, everything else is added. Recorded on `source` as it happens, so the catalogue and the
    /// account of where it came from cannot disagree.
    ///
    /// **A duplicate is judged per folder, then the merged whole is validated once.** Two files in
    /// one folder declaring the same name are still the ambiguity [`Definitions::new`] refuses, and
    /// the error names the folder. Across the two folders the same name is not a duplicate — it is
    /// the shadowing the merge exists for. Everything else (handle form, reserved prefix, a flow
    /// reaching itself) is checked on the MERGED catalogue, because that is the team that runs: a
    /// user-level flow can only form a cycle through the workspace's channels, or be relieved of one
    /// by a workspace channel that hides a step.
    ///
    /// A malformed user-level file fails every workspace's catalogue, with that file's path in the
    /// error — the same fail-closed rule a malformed workspace file has, and for the same reason: a
    /// team half-read is worse than a team refused with the name of the file to fix.
    fn load(mut source: DeclarationSource) -> Result<Definitions> {
        let personas = crate::role::load_declared_personas(&source.read_dir)?;
        let declared_channels = crate::channel::load_declared_channels(&source.read_dir)?;
        let (mut roles, mut files): (Vec<RoleDecl>, Vec<DeclarationFile>) =
            personas.into_iter().map(|p| (p.decl, p.file)).unzip();
        let mut channels = Vec::with_capacity(declared_channels.len());
        for (channel, file) in declared_channels {
            channels.push(channel);
            files.push(file);
        }
        reject_duplicates_in(&source.read_dir, &roles, &channels)?;
        if let Some(user_dir) = source.user_path.clone() {
            let user_personas = crate::role::load_declared_personas(&user_dir)?;
            let user_channels = crate::channel::load_declared_channels(&user_dir)?;
            {
                let roles: Vec<&RoleDecl> = user_personas.iter().map(|p| &p.decl).collect();
                let channels: Vec<&ChannelDecl> = user_channels.iter().map(|(c, _)| c).collect();
                reject_duplicate_refs_in(&user_dir, &roles, &channels)?;
            }
            for persona in user_personas {
                let name = DeclaredName {
                    kind: DeclarationKind::Persona,
                    name: persona.decl.handle.clone(),
                };
                match roles.iter().any(|r| r.handle == persona.decl.handle) {
                    true => source.shadowed.push(name),
                    false => {
                        source.from_user.push(name);
                        roles.push(persona.decl);
                        files.push(persona.file);
                    }
                }
            }
            for (channel, file) in user_channels {
                let name = DeclaredName {
                    kind: DeclarationKind::Channel,
                    name: channel.name.clone(),
                };
                match channels.iter().any(|c| c.name == channel.name) {
                    true => source.shadowed.push(name),
                    false => {
                        source.from_user.push(name);
                        channels.push(channel);
                        files.push(file);
                    }
                }
            }
            // The loader's own contract: roles handle-sorted. Channels keep declaration order,
            // the workspace's first.
            roles.sort_by(|a, b| a.handle.cmp(&b.handle));
        }
        let mut defs = Definitions::new(roles, channels)?;
        source.count = defs.len();
        source.files = files;
        defs.source = Some(source);
        Ok(defs)
    }

    /// Where this catalogue came from — `None` only for one built in memory by
    /// [`Definitions::new`]; see the field's own doc.
    pub fn source(&self) -> Option<&DeclarationSource> {
        self.source.as_ref()
    }

    /// Attach a resolution to a catalogue that was built in memory (nxf 6j6v.n92p).
    ///
    /// The one caller is [`crate::declaration_freeze`], which rebuilds an operation's FROZEN
    /// catalogue out of a stored copy: its declarations are the ones the operation is bound to, and
    /// its resolution — which folder this workspace reads, and how many declarations are in it — is
    /// a fact about the filesystem NOW and belongs to the live catalogue. Carrying the live source
    /// across is what keeps a frozen catalogue from reporting a stale path to a surface that prints
    /// one.
    ///
    /// `pub(crate)`, and deliberately: it exists so ONE construction seam can compose those two
    /// halves, not so a caller can relabel where a catalogue came from.
    pub(crate) fn with_source(mut self, source: Option<DeclarationSource>) -> Definitions {
        self.source = source;
        self
    }

    /// How many declarations this catalogue carries: personas + channels.
    pub fn len(&self) -> usize {
        self.roles.len() + self.channels.len()
    }

    /// Whether NOTHING is declared. The bootstrap question: with no declaration and no raw channels
    /// there is not a single `send --to` target in the workspace (nxf 6j6v.dvyq step 4).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn roles(&self) -> &[RoleDecl] {
        &self.roles
    }

    pub fn channels(&self) -> &[ChannelDecl] {
        &self.channels
    }

    /// Resolve a role by its declared `handle`. Replaces `cli.rs`'s `valid_role_handle` +
    /// `resolve_role` pair, and keeps their error-kind split, which is part of the contract both
    /// seams inherit: a malformed handle is the caller's `validation` mistake; a well-formed handle
    /// naming nothing declared is `not_found`. An app needs to tell "you typed something illegal"
    /// apart from "that role does not exist here".
    ///
    /// **Lookup is by the declared `handle` field, not by a filename.** `resolve_role` used to read
    /// `<roles>/<handle>.yaml` directly, so a file named `bob.yaml` declaring `handle: alice` was
    /// reachable as `bob` — and then `create_pending_session` bound the session to the REQUESTED
    /// handle while the spawned session acted as `decl.handle`, splitting the bound role from the
    /// acting one. Resolving by the declared field is the only thing that means anything for
    /// host-supplied definitions (there are no filenames), and it closes that split.
    pub fn role(&self, handle: &str) -> Result<&RoleDecl> {
        validate_role_handle(handle)?;
        self.roles
            .iter()
            .find(|r| r.handle == handle)
            .ok_or_else(|| NxfError::not_found(format!("no such role: {handle}")))
    }

    /// A plain lookup by channel name, with no validation. This is what a read-shaped caller wants
    /// — `nxc tick` resolves the declared channel backing a thread that already exists, and is
    /// reporting on it rather than acting on it. To act, use [`Definitions::declared_channel`].
    pub fn channel(&self, name: &str) -> Option<&ChannelDecl> {
        self.channels.iter().find(|c| c.name == name)
    }

    /// Resolve a declared channel for a verb that is about to ACT on it — the direct replacement
    /// for `cli.rs`'s `resolve_declared_channel`, including both halves of its contract:
    ///
    /// - an undeclared name is `Ok(None)`, never an error, so a workspace with no declared channels
    ///   falls straight through to the raw-`channel_id` behaviour;
    /// - a declared but MALFORMED channel is a `validation` error rather than a silent degradation
    ///   (`crate::channel::validate_channel_for_use`'s fail-closed contract — a declaration can be
    ///   edited into invalidity after a thread on it was already opened).
    pub fn declared_channel(&self, name: &str) -> Result<Option<&ChannelDecl>> {
        let Some(channel) = self.channel(name) else {
            return Ok(None);
        };
        if let Some(err) = crate::channel::validate_channel_for_use(channel) {
            return Err(NxfError::validation(format!(
                "declared channel {:?} is malformed: {}",
                channel.name, err.what
            )));
        }
        Ok(Some(channel))
    }

    /// Whether `channel`'s chain needs EXCLUSIVE use of the repo's working copy — the WHETHER half
    /// of the working-tree lease (nxf 6j6v.1xw1), read by `orchestration::needs_working_tree`.
    ///
    /// Two ways to be true, and the second is this item's whole first acceptance point:
    ///
    /// 1. the channel says so itself (`working_tree: exclusive` on the declaration);
    /// 2. **a declared MEMBER of it says so.** Owner, 2026-08-14: *"wenn die Kanal-Deklaration in
    ///    seiner Kette eine Persona hat, die einen geschuetzten Arbeitsbereich benoetigt, dann gilt
    ///    der gesamte Kanal als schuetzenswert."* An author who has already declared the need on the
    ///    persona that builds does not repeat it on every channel that persona belongs to.
    ///
    /// **ONE HOP, and never transitive.** What counts is a member's OWN `working_tree:` declaration
    /// — never one that member would inherit from some other channel it also belongs to. The reason
    /// is in the item: without that stop, `#coding` would hand its need for protection to `#review`
    /// through any member the two share, `#review` would hand it on again, and every channel in a
    /// workspace would end up exclusive — including the ones whose members only READ. This function
    /// reads `RoleDecl::working_tree` and nothing else, which is what makes the derivation stop after
    /// one step by construction rather than by a visited set.
    ///
    /// The DECLARED members, not the effective fan-out: `expects: subset` narrows who is asked on a
    /// given turn, but the need for a protected working copy is a property of the channel, and it
    /// must not depend on which members a particular call happens to reach.
    ///
    /// A member naming no declared ROLE contributes nothing, and since nxf 6j6v.hq71 that covers two
    /// cases rather than one. A name that is nothing at all cannot fan out (the fan-out's own
    /// preflight refuses it). A name that is a declared CHANNEL — a flow step addressing a channel —
    /// contributes nothing DELIBERATELY: that is the one-hop rule above, seen from the other side. A
    /// channel does not inherit its neighbour's need for the working copy, whether it reaches that
    /// neighbour through a shared member or by naming it as a step.
    pub fn channel_needs_working_tree(&self, channel: &ChannelDecl) -> bool {
        if channel.working_tree == crate::role::WorkingTree::Exclusive {
            return true;
        }
        // The CAST (nxf 6j6v.g0yn): a step target needing the working copy exclusively makes the
        // channel need it, exactly as a member does — before this, a stepped channel that named
        // its personas only in `steps:` answered `false` and skipped the serialisation it needs.
        crate::channel::cast(channel).iter().any(|handle| {
            self.roles.iter().any(|role| {
                role.handle == *handle && role.working_tree == crate::role::WorkingTree::Exclusive
            })
        })
    }

    // `Definitions::workflow` — the 0/1/N selection rule `workflow start --name` resolved through
    // — stood here. REMOVED with the run engine (6j6v.dvyq §3).
}

/// The role-handle form check, shared by construction and lookup (see [`Definitions::new`] for why
/// each clause is here).
fn validate_role_handle(handle: &str) -> Result<()> {
    if handle.is_empty() || handle.contains('/') || handle.contains('\\') || handle.contains("..") {
        return Err(NxfError::validation(format!(
            "invalid role handle: {handle:?}"
        )));
    }
    // **The whole CLASS, not the three handles that exist today** (nxf 6j6v.pf6j, fix round 1).
    // This used to name `__synth__` and `__delivered__` one by one, and `__channel__` — added by the
    // two-level channel — was documented as reserved without ever being reserved, so a declaration
    // could claim the marker that `supervised_children`, the supervisor's own routing, the lease
    // claim area and the `requester_only` read all key on. Enumerating them again would leave the
    // NEXT marker in the same state: the two items after pf6j each add engine-internal identities,
    // and a list is a thing an author has to remember to extend. The prefix is what every one of
    // them already has, so the rule describes the convention rather than inventing one, and it costs
    // nothing to adopt: no declaration in this repo or its examples uses it.
    if handle.starts_with(RESERVED_HANDLE_PREFIX) {
        return Err(NxfError::validation(format!(
            "role handle {handle:?} is reserved for internal use and cannot be used by a role: \
             handles starting with {RESERVED_HANDLE_PREFIX:?} are the engine's own synthetic \
             identities"
        )));
    }
    Ok(())
}

/// [`reject_duplicates`] for one folder's personas and channels, naming the folder — so a duplicate
/// in the user-level folder is not mistaken for one in the workspace (nxf 6j6v.7k58).
fn reject_duplicates_in(dir: &Path, roles: &[RoleDecl], channels: &[ChannelDecl]) -> Result<()> {
    let roles: Vec<&RoleDecl> = roles.iter().collect();
    let channels: Vec<&ChannelDecl> = channels.iter().collect();
    reject_duplicate_refs_in(dir, &roles, &channels)
}

fn reject_duplicate_refs_in(
    dir: &Path,
    roles: &[&RoleDecl],
    channels: &[&ChannelDecl],
) -> Result<()> {
    reject_duplicates("role handle", roles.iter().map(|r| r.handle.as_str()))
        .and_then(|()| reject_duplicates("channel name", channels.iter().map(|c| c.name.as_str())))
        .map_err(|e| NxfError::validation(format!("{}: {}", dir.display(), e.msg)))
}

/// Reject a repeated name, saying which kind and which value — the ambiguity a caller-supplied
/// `Vec` can express and a one-file-per-name directory could not.
fn reject_duplicates<'a>(what: &str, names: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = HashSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(NxfError::validation(format!(
                "duplicate {what}: {name:?} is declared more than once"
            )));
        }
    }
    Ok(())
}
