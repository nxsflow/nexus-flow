//! Errors as v1 prints them (RFC 0001, "Reading and writing"; RFC 0002, "JSON"): the error object
//! `{"error":{"kind":"<code>","msg":"<message>"}}` on stdout with exit code 1, where `kind` is a
//! stable code an agent matches on and `msg` is for a person. With `--md`, `error: <msg>` goes to
//! stderr instead, as v1's human output does. The codes are v1's.

use nexus_flow_seam::{ConfigError, DataDirError, Error, InitError, Problem, RootError};

use crate::output::Format;
use crate::Outcome;

/// The stable codes of the error object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// No `nexus-flow.toml` from the folder upward: run `nxs init`.
    NoWorkspace,
    /// A setting, a file or an argument is not one `nxs` accepts.
    Validation,
    /// What would be written is already there, and `nxs` does not overwrite it.
    Conflict,
    /// Reading or writing the disk or the database failed.
    Io,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::NoWorkspace => "no_workspace",
            Kind::Validation => "validation",
            Kind::Conflict => "conflict",
            Kind::Io => "io",
        }
    }
}

/// An error `nxs` prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub kind: Kind,
    pub msg: String,
}

impl CliError {
    /// A command line clap could not parse: its first line, without clap's `error: `.
    pub fn usage(e: &clap::Error) -> Self {
        let text = e.render().to_string();
        let line = text.lines().next().unwrap_or_default();
        CliError {
            kind: Kind::Validation,
            msg: line.strip_prefix("error: ").unwrap_or(line).to_owned(),
        }
    }

    /// What the process prints for this error in `format`, and its exit code.
    pub fn outcome(&self, format: Format) -> Outcome {
        match format {
            Format::Json => {
                let object = serde_json::json!({
                    "error": { "kind": self.kind.as_str(), "msg": self.msg }
                });
                Outcome {
                    stdout: format!("{object}\n").into_bytes(),
                    stderr: Vec::new(),
                    code: 1,
                }
            }
            Format::Md => Outcome {
                stdout: Vec::new(),
                stderr: format!("error: {}\n", self.msg).into_bytes(),
                code: 1,
            },
        }
    }
}

impl From<DataDirError> for CliError {
    fn from(e: DataDirError) -> Self {
        CliError {
            kind: Kind::Validation,
            msg: e.to_string(),
        }
    }
}

impl From<Error> for CliError {
    fn from(e: Error) -> Self {
        let msg = e.to_string();
        let kind = match &e {
            Error::DataDir(_) => Kind::Validation,
            Error::Open(open) => open_kind(open),
        };
        CliError { kind, msg }
    }
}

fn open_kind(e: &InitError) -> Kind {
    match e {
        InitError::Root(RootError::NotFound { .. }) => Kind::NoWorkspace,
        InitError::Root(RootError::DataDirInside { .. }) => Kind::Validation,
        InitError::Config(ConfigError { problem, .. }) => match problem {
            Problem::AlreadyNamesSpace { .. } | Problem::AlreadyExists => Kind::Conflict,
            Problem::Io { .. } => Kind::Io,
            _ => Kind::Validation,
        },
        InitError::DataDir { .. } => Kind::Io,
        InitError::SpaceId { .. } => Kind::Validation,
        InitError::Database { .. } => Kind::Io,
    }
}
