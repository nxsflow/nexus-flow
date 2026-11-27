//! The genesis module's storage (RFC 0004): a repository's GENESIS is the file
//! `factory/GENESIS.md` in git, and the file is the truth. `create`, the seam's
//! `Engine::genesis_create`, writes it from the template nexus-flow ships, only where the path is
//! free ("Installing the template"):
//!
//! - `written`: nothing is at the path, and git tracks it neither in the index, in `HEAD` nor on
//!   the base branch; the template is written with an exclusive create, which fails on anything
//!   that appeared at the path since, a symbolic link included, dangling or not.
//! - `team`: anything is at the path, whatever its bytes, the template's included, or a file whose
//!   name differs from `GENESIS.md` only in case is in `factory/`; nothing is written, ever.
//! - `tracked`: the path is missing but git still tracks it; nothing is written, and the report
//!   is refused, so that `nxs` exits non-zero and says to restore the file from git.
//!
//! Beside GENESIS, `create` installs the guide, `factory/genesis-guide.md`, under RFC 0006's rules
//! for shipped defaults (see [`install_guide`]), and reports which of the paths CODEOWNERS leaves
//! without a code owner: GENESIS only, the guide is not checked.
//!
//! It never commits, never opens a NoCentralDB database, and `update` never touches GENESIS: it
//! applies to the guide alone.

mod git;
mod guide;

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use nexus_flow_store::config::{Config, ConfigError};
use nexus_flow_store::repo_root::{RepoRoot, RootError};

pub use git::GitError;
pub use guide::{install_guide, GUIDE, GUIDE_PATH, SHIPPED};

use crate::install::codeowners;
use crate::install::shipped::{Shipped, ShippedError, State};

/// The template nexus-flow writes as a new GENESIS, embedded at build time.
pub const TEMPLATE: &str = include_str!("../../../../templates/genesis/GENESIS.md");

/// The mark on the line before the list of rules on what comes back to the owner (RFC 0004, "The
/// rules on what comes back to the owner").
pub const OWNER_RULES_MARK: &str = "<!-- nexus-flow: owner-rules -->";

/// The folder of GENESIS, relative to the repository's root.
const FOLDER: &str = "factory";
/// The file name of GENESIS in it.
const FILE: &str = "GENESIS.md";
/// The path of GENESIS, relative to the repository's root.
pub const PATH: &str = "factory/GENESIS.md";

/// What `create` found at `factory/GENESIS.md`, and what it did there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenesisState {
    /// Nothing was there and git tracked nothing: the template was written.
    Written,
    /// Something was there, or a case variant of it: nothing was written.
    Team,
    /// Missing, but tracked in git: nothing was written, and the run is refused.
    Tracked,
}

impl GenesisState {
    /// The state's name in the report.
    pub fn as_str(self) -> &'static str {
        match self {
            GenesisState::Written => "written",
            GenesisState::Team => "team",
            GenesisState::Tracked => "tracked",
        }
    }
}

/// The state of one file of the report: GENESIS has its own three, the guide RFC 0006's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// The state of `factory/GENESIS.md`.
    Genesis(GenesisState),
    /// The state of the guide, a shipped default.
    Shipped(State),
}

impl FileState {
    /// The state's name in the report.
    pub fn as_str(self) -> &'static str {
        match self {
            FileState::Genesis(state) => state.as_str(),
            FileState::Shipped(state) => state.as_str(),
        }
    }
}

/// One file of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    /// The path, relative to the repository's root.
    pub path: String,
    /// What was found there, and done.
    pub state: FileState,
    /// For a case variant, the path as it is on disk.
    pub found: Option<String>,
}

/// What `create` reports: its files, ordered by path, bytewise, and the CODEOWNERS report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub files: Vec<FileReport>,
    /// `None` when the repository has no CODEOWNERS file.
    pub codeowners: Option<codeowners::Report>,
}

impl Report {
    /// Whether a file is `tracked`: the caller exits non-zero, with the report.
    pub fn is_refused(&self) -> bool {
        self.files
            .iter()
            .any(|f| f.state == FileState::Genesis(GenesisState::Tracked))
    }
}

/// Why `create` could not run.
#[derive(Debug)]
pub enum GenesisError {
    /// No `nexus-flow.toml` from the folder upward.
    Root(RootError),
    /// `nexus-flow.toml` could not be read.
    Config(ConfigError),
    /// Reading or writing a file failed.
    Io { path: PathBuf, source: io::Error },
    /// git could not say whether it tracks GENESIS.
    Git(GitError),
    /// The guide's shipped list cannot be read: a bug in this release.
    Shipped(ShippedError),
}

impl fmt::Display for GenesisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GenesisError::Root(e) => e.fmt(f),
            GenesisError::Config(e) => e.fmt(f),
            GenesisError::Io { path, source } => write!(f, "{}: {source}", path.display()),
            GenesisError::Git(GitError::Spawn(e)) => {
                write!(f, "git could not be started to check {PATH}: {e}")
            }
            GenesisError::Git(GitError::Failed { args, stderr }) => {
                write!(f, "git {args} failed while checking {PATH}: {stderr}")
            }
            GenesisError::Shipped(e) => write!(f, "the genesis guide's {e}"),
        }
    }
}

impl std::error::Error for GenesisError {}

/// Writes `factory/GENESIS.md` from the template for the repository at or above `dir`, only where
/// the path is free, installs the guide, and reports CODEOWNERS. `update` applies to the guide
/// only, never to GENESIS. GENESIS and the guide are handled each on its own: a `tracked` GENESIS
/// does not keep the guide from being installed.
pub fn create(dir: &Path, update: bool) -> Result<Report, GenesisError> {
    let root = RepoRoot::find(dir).map_err(GenesisError::Root)?;
    let config = Config::read(&root).map_err(GenesisError::Config)?;
    let root = root.path();
    let mut files = vec![genesis(root, &config.base)?];
    let shipped = Shipped::parse(SHIPPED).map_err(GenesisError::Shipped)?;
    files.extend(install_guide(root, &shipped, GUIDE, update)?);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let codeowners = codeowners::report(root, &[PATH]).map_err(|source| GenesisError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    Ok(Report { files, codeowners })
}

/// The state of GENESIS under `root`, writing the template where the path is free.
fn genesis(root: &Path, base: &str) -> Result<FileReport, GenesisError> {
    let report = |state, found| FileReport {
        path: PATH.to_owned(),
        state: FileState::Genesis(state),
        found,
    };
    let folder = root.join(FOLDER);
    if let Some(name) = present(&folder)? {
        let found = (name != FILE).then(|| format!("{FOLDER}/{name}"));
        return Ok(report(GenesisState::Team, found));
    }
    if git::tracked(root, PATH, base).map_err(GenesisError::Git)? {
        return Ok(report(GenesisState::Tracked, None));
    }
    fs::create_dir_all(&folder).map_err(|source| GenesisError::Io {
        path: folder.clone(),
        source,
    })?;
    let path = folder.join(FILE);
    let io = |source| GenesisError::Io {
        path: path.clone(),
        source,
    };
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(TEMPLATE.as_bytes()).map_err(io)?;
            Ok(report(GenesisState::Written, None))
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(report(GenesisState::Team, None)),
        Err(e) => Err(io(e)),
    }
}

/// The name of the entry in `folder` that is GENESIS: `GENESIS.md` itself, whatever kind of
/// entry, a symbolic link included, or else the first, bytewise, whose name differs from it only
/// in case. `None` when `folder` holds neither, or does not exist.
fn present(folder: &Path) -> Result<Option<String>, GenesisError> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(GenesisError::Io {
                path: folder.to_path_buf(),
                source,
            })
        }
    };
    let mut variants = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| GenesisError::Io {
            path: folder.to_path_buf(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == FILE {
            return Ok(Some(name));
        }
        if name.to_lowercase() == FILE.to_lowercase() {
            variants.push(name);
        }
    }
    variants.sort();
    Ok(variants.into_iter().next())
}
