//! `nexus-flow.toml` (RFC 0001, "Creating and finding a space").

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::repo_root::RepoRoot;

/// What the file is wrong about, or why it was not written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Syntax { message: String },
    UnsupportedFormat { found: String },
    Missing { key: &'static str },
    Empty { key: &'static str },
    NotAString { key: &'static str },
    NotATable { key: &'static str },
    UnknownKey { key: String },
    AlreadyNamesSpace { repository: String },
    AlreadyExists,
    Io { kind: io::ErrorKind, message: String },
}

/// A problem with one `nexus-flow.toml`, naming the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub file: PathBuf,
    pub problem: Problem,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

impl std::error::Error for ConfigError {}

/// The contents of `nexus-flow.toml`, format 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub repository: String,
    pub coordination: Option<String>,
    pub base: String,
}

impl Config {
    pub fn new(repository: impl Into<String>) -> Self {
        Config {
            repository: repository.into(),
            coordination: None,
            base: String::new(),
        }
    }

    pub fn parse(_text: &str, file: &Path) -> Result<Self, ConfigError> {
        Err(ConfigError {
            file: file.to_path_buf(),
            problem: Problem::AlreadyExists,
        })
    }

    pub fn read(root: &RepoRoot) -> Result<Self, ConfigError> {
        Self::parse("", &root.config_file())
    }

    pub fn to_toml(&self) -> String {
        String::new()
    }

    pub fn write_new(&self, dir: &Path) -> Result<(), ConfigError> {
        Err(ConfigError {
            file: dir.to_path_buf(),
            problem: Problem::AlreadyExists,
        })
    }
}
