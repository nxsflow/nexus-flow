//! `nexus-flow.toml` (RFC 0001, "Creating and finding a space"): the committed file at the
//! repository's root that names the repository's space, its coordination space if any, and the
//! team's base branch. It holds no secret: a space id grants nothing.
//!
//! ```toml
//! format = 1
//!
//! [space]
//! repository = "<NoCentralDB space id>"
//! coordination = "<NoCentralDB space id>"   # optional
//!
//! [git]
//! base = "main"                             # optional, main when unset
//! ```
//!
//! Only format 1 is read: another format is refused, never guessed at, and so is a key format 1
//! does not know. Writing creates the file and never overwrites one that is already there.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::repo_root::{RepoRoot, FILE_NAME};

/// The one format this module reads and writes.
const FORMAT: i64 = 1;

/// The base branch when `[git] base` is unset.
const DEFAULT_BASE: &str = "main";

/// The comment the written file opens with.
const HEADER: &str = "# nexus-flow.toml, committed. Holds no secret: a space id grants nothing.";

/// What the file is wrong about, or why it was not written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The text is not TOML.
    Syntax { message: String },
    /// `format` is not 1; `found` is its value as written.
    UnsupportedFormat { found: String },
    /// A required key is missing.
    Missing { key: &'static str },
    /// A space id or the base branch is an empty string.
    Empty { key: &'static str },
    /// A key that holds a string holds something else.
    NotAString { key: &'static str },
    /// `space` or `git` is not a table.
    NotATable { key: &'static str },
    /// A key format 1 does not know.
    UnknownKey { key: String },
    /// Writing found a file that already names this repository space.
    AlreadyNamesSpace { repository: String },
    /// Writing found a file that names no repository space.
    AlreadyExists,
    /// The file could not be read or written.
    Io { kind: io::ErrorKind, message: String },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Syntax { message } => write!(f, "not valid TOML: {message}"),
            Problem::UnsupportedFormat { found } => write!(
                f,
                "format {found} is not one this nxs reads; it reads format {FORMAT}"
            ),
            Problem::Missing { key } => write!(f, "{key} is missing"),
            Problem::Empty { key } => write!(f, "{key} is empty"),
            Problem::NotAString { key } => write!(f, "{key} must be a string"),
            Problem::NotATable { key } => write!(f, "{key} must be a table"),
            Problem::UnknownKey { key } => {
                write!(f, "{key} is not a key of format {FORMAT}")
            }
            Problem::AlreadyNamesSpace { repository } => write!(
                f,
                "already names the repository space {repository}; nxs never overwrites it"
            ),
            Problem::AlreadyExists => write!(f, "already exists; nxs never overwrites it"),
            Problem::Io { message, .. } => f.write_str(message),
        }
    }
}

/// A problem with one `nexus-flow.toml`, naming the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub file: PathBuf,
    pub problem: Problem,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.file.display(), self.problem)
    }
}

impl std::error::Error for ConfigError {}

/// The contents of `nexus-flow.toml`, format 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The repository space's id.
    pub repository: String,
    /// The coordination space's id, once the repository has joined one.
    pub coordination: Option<String>,
    /// The team's base branch, which "on main" is checked against.
    pub base: String,
}

impl Config {
    /// A file that names only the repository space, with the default base branch.
    pub fn new(repository: impl Into<String>) -> Self {
        Config {
            repository: repository.into(),
            coordination: None,
            base: DEFAULT_BASE.to_owned(),
        }
    }

    /// Parses the text of the file at `file`, which every error names.
    pub fn parse(text: &str, file: &Path) -> Result<Self, ConfigError> {
        let error = |problem| ConfigError {
            file: file.to_path_buf(),
            problem,
        };
        let table: Table = text.parse().map_err(|e: toml::de::Error| {
            error(Problem::Syntax {
                message: e.message().to_owned(),
            })
        })?;
        Self::from_table(&table).map_err(error)
    }

    /// Reads the root's `nexus-flow.toml`.
    pub fn read(root: &RepoRoot) -> Result<Self, ConfigError> {
        let file = root.config_file();
        let text = fs::read_to_string(&file).map_err(|e| ConfigError {
            file: file.clone(),
            problem: io_problem(&e),
        })?;
        Self::parse(&text, &file)
    }

    /// The file's text, format 1. A coordination space that is unset is left out.
    pub fn to_toml(&self) -> String {
        let quote = |s: &str| Value::String(s.to_owned()).to_string();
        let mut text = format!(
            "{HEADER}\nformat = {FORMAT}\n\n[space]\nrepository = {}\n",
            quote(&self.repository)
        );
        if let Some(coordination) = &self.coordination {
            text.push_str(&format!("coordination = {}\n", quote(coordination)));
        }
        text.push_str(&format!("\n[git]\nbase = {}\n", quote(&self.base)));
        text
    }

    /// Writes `nexus-flow.toml` into `dir`, which must not hold one yet. A file that is already
    /// there is left byte for byte as it was, and the error says whether it names a repository
    /// space.
    pub fn write_new(&self, dir: &Path) -> Result<(), ConfigError> {
        let file = dir.join(FILE_NAME);
        let error = |problem| ConfigError {
            file: file.clone(),
            problem,
        };
        let mut out = match OpenOptions::new().write(true).create_new(true).open(&file) {
            Ok(out) => out,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(error(existing_problem(&file)));
            }
            Err(e) => return Err(error(io_problem(&e))),
        };
        out.write_all(self.to_toml().as_bytes())
            .and_then(|()| out.sync_all())
            .map_err(|e| error(io_problem(&e)))
    }

    fn from_table(table: &Table) -> Result<Self, Problem> {
        // The format is checked first, so that a future format is refused as such and not for
        // a key format 1 does not know.
        match table.get("format") {
            None => return Err(Problem::Missing { key: "format" }),
            Some(Value::Integer(FORMAT)) => {}
            Some(other) => {
                return Err(Problem::UnsupportedFormat {
                    found: other.to_string(),
                })
            }
        }
        only_keys(table, "", &["format", "space", "git"])?;
        let space = subtable(table, "space")?;
        let git = subtable(table, "git")?;
        if let Some(space) = space {
            only_keys(space, "space.", &["repository", "coordination"])?;
        }
        if let Some(git) = git {
            only_keys(git, "git.", &["base"])?;
        }

        let repository = string(space, "repository", "space.repository")?.ok_or(
            Problem::Missing {
                key: "space.repository",
            },
        )?;
        let coordination = string(space, "coordination", "space.coordination")?;
        let base = string(git, "base", "git.base")?.unwrap_or_else(|| DEFAULT_BASE.to_owned());
        Ok(Config {
            repository,
            coordination,
            base,
        })
    }
}

/// Refuses the first key of `table` that is not in `known`; `prefix` places it in the file.
fn only_keys(table: &Table, prefix: &str, known: &[&str]) -> Result<(), Problem> {
    match table.keys().find(|key| !known.contains(&key.as_str())) {
        Some(key) => Err(Problem::UnknownKey {
            key: format!("{prefix}{key}"),
        }),
        None => Ok(()),
    }
}

fn subtable<'a>(table: &'a Table, key: &'static str) -> Result<Option<&'a Table>, Problem> {
    match table.get(key) {
        None => Ok(None),
        Some(Value::Table(inner)) => Ok(Some(inner)),
        Some(_) => Err(Problem::NotATable { key }),
    }
}

/// The non-empty string at `key` of `table`, if both are there; `path` names it in an error.
fn string(
    table: Option<&Table>,
    key: &str,
    path: &'static str,
) -> Result<Option<String>, Problem> {
    match table.and_then(|t| t.get(key)) {
        None => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Err(Problem::Empty { key: path }),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(Problem::NotAString { key: path }),
    }
}

/// Why an existing file is not overwritten: the repository space it names, whatever its format,
/// or only that it exists.
fn existing_problem(file: &Path) -> Problem {
    let named = fs::read_to_string(file)
        .ok()
        .and_then(|text| text.parse::<Table>().ok())
        .and_then(|table| match table.get("space")?.get("repository")? {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        });
    match named {
        Some(repository) => Problem::AlreadyNamesSpace { repository },
        None => Problem::AlreadyExists,
    }
}

fn io_problem(e: &io::Error) -> Problem {
    Problem::Io {
        kind: e.kind(),
        message: e.to_string(),
    }
}
