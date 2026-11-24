//! Installing the factory's shipped defaults into a repository's `factory/` (RFC 0006, "The
//! factory's files"; "Installing the shipped defaults").
//!
//! [`install`] reads only the paths the shipped list names, gives each its state, reads the
//! CODEOWNERS file, and only then writes: the current default where a file is missing, and with
//! `update` where a file is an earlier default. It never deletes, never writes a path the list
//! does not name or a retired one, never touches a team's file, and never writes CODEOWNERS. A
//! read that fails writes nothing.
//!
//! A retired path whose file is missing has no state and is left out of the report, until the
//! owner answers whether the report lists it (NEX-235).

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use nexus_flow_store::repo_root::{RepoRoot, RootError};

use super::codeowners;
use super::embedded;
use super::shipped::{sha256_lf, Shipped, State};

/// The folder at the repository's root that holds the factory (RFC 0006, "The path is fixed").
pub const FOLDER: &str = "factory";

/// The shipped list and the current default's text for each path it ships.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    shipped: Shipped,
    texts: BTreeMap<String, Vec<u8>>,
}

/// Why a set of defaults does not hold together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultsError {
    /// A path the list ships, not retired, has no text.
    Missing(String),
    /// A text is given for a path the list does not name, or names as retired.
    NotShipped(String),
    /// A path's text is not the version the list names as current.
    NotCurrent(String),
}

impl fmt::Display for DefaultsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DefaultsError::Missing(p) => write!(f, "the shipped list names {p}, which has no text"),
            DefaultsError::NotShipped(p) => {
                write!(f, "{p} has a text but the shipped list does not ship it")
            }
            DefaultsError::NotCurrent(p) => {
                write!(
                    f,
                    "the text of {p} is not the version the shipped list names current"
                )
            }
        }
    }
}

impl std::error::Error for DefaultsError {}

impl Defaults {
    /// The defaults `list` ships, with `texts` giving the current text of each path, relative to
    /// `factory/`. Every path the list ships and has not retired needs exactly its current text.
    pub fn new<P: Into<String>>(
        shipped: Shipped,
        texts: impl IntoIterator<Item = (P, Vec<u8>)>,
    ) -> Result<Defaults, DefaultsError> {
        let texts: BTreeMap<String, Vec<u8>> =
            texts.into_iter().map(|(p, t)| (p.into(), t)).collect();
        for (path, text) in &texts {
            let Some(current) = shipped.current(path).filter(|_| !shipped.is_retired(path)) else {
                return Err(DefaultsError::NotShipped(path.clone()));
            };
            if sha256_lf(text) != current {
                return Err(DefaultsError::NotCurrent(path.clone()));
            }
        }
        if let Some(path) = shipped
            .paths()
            .find(|p| !shipped.is_retired(p) && !texts.contains_key(*p))
        {
            return Err(DefaultsError::Missing(path.to_string()));
        }
        Ok(Defaults { shipped, texts })
    }

    /// The defaults this `nxs` embeds from `templates/factory/`.
    pub fn embedded() -> Defaults {
        let shipped = Shipped::parse(embedded::SHIPPED).expect("the embedded shipped.toml reads");
        let texts = embedded::TEXTS.iter().map(|&(p, t)| (p, t.to_vec()));
        Defaults::new(shipped, texts).expect("the embedded defaults hold together")
    }

    /// The shipped list.
    pub fn shipped(&self) -> &Shipped {
        &self.shipped
    }

    /// The current default's text at `path`, relative to `factory/`; `None` for a path that is
    /// not shipped or is retired.
    pub fn text(&self, path: &str) -> Option<&[u8]> {
        self.texts.get(path).map(Vec::as_slice)
    }
}

/// One file in `install`'s report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    /// The path relative to the repository's root, such as `factory/roles/pm.md`.
    pub path: String,
    /// What install found there, or did.
    pub state: State,
}

/// What `install` reports: the shipped files in bytewise order of their paths, and the
/// CODEOWNERS file with the reported paths no rule covers, `None` without a CODEOWNERS file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub files: Vec<FileReport>,
    pub codeowners: Option<codeowners::Report>,
}

/// A file install could not read or write.
#[derive(Debug)]
pub struct InstallError {
    /// The file, or for CODEOWNERS the repository's root.
    pub path: PathBuf,
    pub source: io::Error,
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.source)
    }
}

impl std::error::Error for InstallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Why the factory's defaults could not be installed into a repository.
#[derive(Debug)]
pub enum FactoryError {
    /// No `nexus-flow.toml` was found from the folder upward.
    Root(RootError),
    /// A file could not be read or written.
    Install(InstallError),
}

impl fmt::Display for FactoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FactoryError::Root(e) => e.fmt(f),
            FactoryError::Install(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for FactoryError {}

/// Installs the embedded defaults into the `factory/` of the repository at or above `dir`, the
/// folder that holds `nexus-flow.toml`. It opens no database.
pub fn install_in(dir: &Path, update: bool) -> Result<Report, FactoryError> {
    let root = RepoRoot::find(dir).map_err(FactoryError::Root)?;
    install(root.path(), &Defaults::embedded(), update).map_err(FactoryError::Install)
}

/// Installs `defaults` into the `factory/` of the repository at `root`, and reports each shipped
/// path's state and the CODEOWNERS gaps.
pub fn install(root: &Path, defaults: &Defaults, update: bool) -> Result<Report, InstallError> {
    let factory = root.join(FOLDER);
    let mut found = Vec::new();
    for path in defaults.shipped.paths() {
        let file = factory.join(path);
        let bytes = read(&file)?;
        if let Some(state) = defaults.shipped.state(path, bytes.as_deref(), update) {
            found.push((path, file, state));
        }
    }
    let files: Vec<FileReport> = found
        .iter()
        .map(|(path, _, state)| FileReport {
            path: format!("{FOLDER}/{path}"),
            state: *state,
        })
        .collect();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let codeowners = codeowners::report(root, &paths).map_err(|source| InstallError {
        path: root.to_path_buf(),
        source,
    })?;
    for (path, file, state) in &found {
        let Some(text) = defaults.text(path) else {
            continue;
        };
        let written = match state {
            State::Written => create(file, text),
            State::Updated => fs::write(file, text),
            _ => continue,
        };
        written.map_err(|source| InstallError {
            path: file.clone(),
            source,
        })?;
    }
    Ok(Report { files, codeowners })
}

/// The bytes at `file`, or `None` when nothing is there; a link counts as something there.
fn read(file: &Path) -> Result<Option<Vec<u8>>, InstallError> {
    let error = |source| InstallError {
        path: file.to_path_buf(),
        source,
    };
    match fs::symlink_metadata(file) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(e)),
        Ok(_) => {}
    }
    fs::read(file).map(Some).map_err(error)
}

/// Writes `text` to the new file `file`, creating its folders; a file that appeared since it was
/// read is not overwritten.
fn create(file: &Path, text: &[u8]) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)?
        .write_all(text)
}
