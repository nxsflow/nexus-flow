//! Recording the repository's coordination space (RFC 0001, "Creating and finding a space";
//! "Spaces": a repository belongs to at most one coordination space): the store half of
//! `nxs space join-coordination <space id>`.
//!
//! `join` writes `[space] coordination` into the `nexus-flow.toml` of the repository at or above
//! a folder. It opens no database: the file is what every clone reads. The line goes right after
//! `repository` in `[space]`, and every other byte of the file stays as the team wrote it, its
//! comments included. Before the file is replaced, the edited text is read back and must name
//! exactly what the old one did plus the coordination space; a layout where the line cannot be
//! placed so is refused, and the error says what to add by hand.
//!
//! A file that already names this coordination space is left untouched; one that names another
//! is refused, since a repository belongs to at most one coordination space and nxs never
//! overwrites a team's configuration. The repository's own space is refused as its coordination
//! space.
//!
//! Leaving takes the line out again the same way: only the `coordination` line of `[space]` goes,
//! and the edited text must name exactly what the old one did without the coordination space.
//!
//! `crate::requests` plans these edits before it writes the repository record (RFC 0009,
//! "Repositories in the space"), and writes the file after it, so a refusal writes nothing.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::config::{Config, ConfigError, Problem};
use crate::id::SpaceId;
use crate::repo_root::{RepoRoot, RootError};

/// What `join` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinReport {
    /// The coordination space `nexus-flow.toml` names now.
    pub coordination: SpaceId,
    /// Whether this run wrote the file; `false` when it already named this space.
    pub written: bool,
}

/// Why the coordination space was not recorded. Nothing was written.
#[derive(Debug)]
pub enum JoinError {
    /// No `nexus-flow.toml` was found from the folder upward.
    Root(RootError),
    /// `nexus-flow.toml` could not be read or written, or is not one this nxs reads.
    Config(ConfigError),
    /// The id is the repository space's own.
    OwnSpace { file: PathBuf, space: SpaceId },
    /// The file already names another coordination space.
    OtherCoordination { file: PathBuf, recorded: String },
    /// The file's layout leaves no place for the line that keeps the rest as it is.
    CannotPlace { file: PathBuf, space: SpaceId },
    /// The file names the coordination space in a layout nxs cannot take the line out of.
    CannotRemove { file: PathBuf, space: String },
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JoinError::Root(e) => e.fmt(f),
            JoinError::Config(e) => e.fmt(f),
            JoinError::OwnSpace { file, space } => write!(
                f,
                "{}: {space} is the repository space this file names, not a coordination space",
                file.display()
            ),
            JoinError::OtherCoordination { file, recorded } => write!(
                f,
                "{}: already names the coordination space {recorded}; a repository belongs to at most one, and nxs never overwrites it",
                file.display()
            ),
            JoinError::CannotPlace { file, space } => write!(
                f,
                "{}: nxs cannot add the coordination space to this file's layout; add coordination = \"{space}\" under [space] by hand",
                file.display()
            ),
            JoinError::CannotRemove { file, space } => write!(
                f,
                "{}: nxs cannot take the coordination space {space} out of this file's layout; remove its line under [space] by hand",
                file.display()
            ),
        }
    }
}

impl std::error::Error for JoinError {}

impl From<RootError> for JoinError {
    fn from(e: RootError) -> Self {
        JoinError::Root(e)
    }
}

impl From<ConfigError> for JoinError {
    fn from(e: ConfigError) -> Self {
        JoinError::Config(e)
    }
}

/// Records `coordination` in the `nexus-flow.toml` of the repository at or above `dir`.
pub fn join(dir: &Path, coordination: SpaceId) -> Result<JoinReport, JoinError> {
    let root = RepoRoot::find(dir)?;
    let edit = plan_join(&root, coordination)?;
    let written = edit.is_some();
    if let Some(edit) = edit {
        edit.apply()?;
    }
    Ok(JoinReport {
        coordination,
        written,
    })
}

/// A new text for `nexus-flow.toml`, checked and not yet written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    file: PathBuf,
    text: String,
}

impl Edit {
    /// Replaces the file with the edited text.
    pub(crate) fn apply(self) -> Result<(), JoinError> {
        replace(&self.file, &self.text).map_err(|e| io_error(&self.file, &e))
    }
}

/// The file and its parsed contents at `root`.
fn read(root: &RepoRoot) -> Result<(PathBuf, String, Config), JoinError> {
    let file = root.config_file();
    let text = fs::read_to_string(&file).map_err(|e| io_error(&file, &e))?;
    let config = Config::parse(&text, &file)?;
    Ok((file, text, config))
}

/// The edit that records `coordination` in the `nexus-flow.toml` at `root`; `None` when the file
/// names it already. Refuses the repository's own space, another coordination space, and a
/// layout the line cannot be placed in.
pub(crate) fn plan_join(root: &RepoRoot, coordination: SpaceId) -> Result<Option<Edit>, JoinError> {
    let (file, text, config) = read(root)?;
    let id = coordination.to_string();
    if config.repository == id {
        return Err(JoinError::OwnSpace {
            file,
            space: coordination,
        });
    }
    match &config.coordination {
        Some(recorded) if *recorded == id => return Ok(None),
        Some(recorded) => {
            return Err(JoinError::OtherCoordination {
                file,
                recorded: recorded.clone(),
            })
        }
        None => {}
    }
    let expected = Config {
        coordination: Some(id.clone()),
        ..config
    };
    let edited = with_coordination(&text, &id)
        .filter(|edited| Config::parse(edited, &file).ok().as_ref() == Some(&expected))
        .ok_or_else(|| JoinError::CannotPlace {
            file: file.clone(),
            space: coordination,
        })?;
    Ok(Some(Edit { file, text: edited }))
}

/// The coordination space the `nexus-flow.toml` at `root` names, and the edit that takes it out;
/// `None` when the file names none. Refuses a layout the line cannot be taken out of.
pub(crate) fn plan_leave(root: &RepoRoot) -> Result<Option<(String, Edit)>, JoinError> {
    let (file, text, config) = read(root)?;
    let Some(recorded) = config.coordination.clone() else {
        return Ok(None);
    };
    let expected = Config {
        coordination: None,
        ..config
    };
    let edited = without_coordination(&text)
        .filter(|edited| Config::parse(edited, &file).ok().as_ref() == Some(&expected))
        .ok_or_else(|| JoinError::CannotRemove {
            file: file.clone(),
            space: recorded.clone(),
        })?;
    Ok(Some((recorded, Edit { file, text: edited })))
}

/// `text` without the `coordination` line of `[space]`; `None` when that section has no such
/// line.
fn without_coordination(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let (header, section_end) = space_section(&lines)?;
    let line = lines[header + 1..section_end]
        .iter()
        .position(|line| is_key(line, "coordination"))
        .map(|i| header + 1 + i)?;
    let mut out = String::with_capacity(text.len());
    for (i, kept) in lines.iter().enumerate() {
        if i != line {
            out.push_str(kept);
        }
    }
    Some(out)
}

/// The index of the `[space]` header in `lines`, and the index of the line after its section.
fn space_section(lines: &[&str]) -> Option<(usize, usize)> {
    let header = lines.iter().position(|line| is_header(line, "space"))?;
    let section_end = lines[header + 1..]
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .map_or(lines.len(), |i| header + 1 + i);
    Some((header, section_end))
}

/// `text` with `coordination = "<id>"` after the `repository` line of `[space]`, or right after
/// the `[space]` header when that section has no such line; `None` when it has no `[space]`
/// header. The line ends as the file's lines do.
fn with_coordination(text: &str, id: &str) -> Option<String> {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let (header, section_end) = space_section(&lines)?;
    let after = lines[header + 1..section_end]
        .iter()
        .position(|line| is_key(line, "repository"))
        .map_or(header, |i| header + 1 + i);
    let mut out = String::with_capacity(text.len() + id.len() + 20);
    for (i, line) in lines.iter().enumerate() {
        out.push_str(line);
        if i == after {
            if !line.ends_with('\n') {
                out.push_str(newline);
            }
            out.push_str(&format!("coordination = \"{id}\"{newline}"));
        }
    }
    Some(out)
}

/// Whether `line` is the table header `[name]`, with an optional comment after it.
fn is_header(line: &str, name: &str) -> bool {
    let code = line.split('#').next().unwrap_or_default().trim();
    code.strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .is_some_and(|inner| inner.trim() == name)
}

/// Whether `line` sets the bare key `key`.
fn is_key(line: &str, key: &str) -> bool {
    line.trim_start()
        .strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// Replaces `file` with `text`: written beside it, with its permissions, then renamed over it,
/// so a reader sees the old file or the new one, never part of one.
fn replace(file: &Path, text: &str) -> io::Result<()> {
    let permissions = fs::metadata(file)?.permissions();
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    let temp = file.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let written = (|| {
        let mut out = fs::File::create(&temp)?;
        out.write_all(text.as_bytes())?;
        out.sync_all()?;
        fs::set_permissions(&temp, permissions)?;
        fs::rename(&temp, file)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

fn io_error(file: &Path, e: &io::Error) -> JoinError {
    JoinError::Config(ConfigError {
        file: file.to_path_buf(),
        problem: Problem::Io {
            kind: e.kind(),
            message: e.to_string(),
        },
    })
}
