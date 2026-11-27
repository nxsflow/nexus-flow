//! Errors as v1 prints them (RFC 0001, "Reading and writing"; RFC 0002, "JSON"): the error object
//! `{"error":{"kind":"<code>","msg":"<message>"}}` on stdout with exit code 1, where `kind` is a
//! stable code an agent matches on and `msg` is for a person. With `--md`, `error: <msg>` goes to
//! stderr instead, as v1's human output does. The codes are v1's, `no_key_store` (NEX-224) and
//! `timeout` (NEX-102); the member commands use v1's `forbidden` and `not_found` (NEX-99), and
//! `no_free_tag` from RFC 0002 (NEX-108).

use nexus_flow_seam::{
    ConfigError, DataDirError, Error, InitError, MemberError, Problem, RootError, SyncError,
};

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
    /// The machine has no key store, where `nxs` keeps the device's keys (NEX-224).
    NoKeyStore,
    /// `nxs sync --timeout` passed while something was still behind or unconfirmed (NEX-102).
    Timeout,
    /// This member may not do it, such as a member that is not an admin adding or removing one.
    Forbidden,
    /// What the command names does not exist, such as a member that is not one.
    NotFound,
    /// Every one of the 936 ticket tags is held, so `nxs member add` has none to give (RFC 0002).
    NoFreeTag,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::NoWorkspace => "no_workspace",
            Kind::Validation => "validation",
            Kind::Conflict => "conflict",
            Kind::Io => "io",
            Kind::NoKeyStore => "no_key_store",
            Kind::Timeout => "timeout",
            Kind::Forbidden => "forbidden",
            Kind::NotFound => "not_found",
            Kind::NoFreeTag => "no_free_tag",
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
            Error::DataDir(_) | Error::Session(_) => Kind::Validation,
            Error::Open(open) => open_kind(open),
            Error::Sync(SyncError::TimedOut(_)) => Kind::Timeout,
            Error::Sync(SyncError::NoRelay) => Kind::Validation,
            Error::Sync(SyncError::Open(open)) => open_kind(open),
            Error::Member(member) => member_kind(member),
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
        InitError::KeyStoreUnavailable => Kind::NoKeyStore,
        InitError::Settings(_) => Kind::Validation,
        InitError::Database { .. } => Kind::Io,
        // NXS_SESSION names a session that is not running on this device: a setting nxs refuses.
        InitError::NoSession { .. } => Kind::Validation,
    }
}

fn member_kind(e: &MemberError) -> Kind {
    match e {
        MemberError::NotAdmin { .. }
        | MemberError::NotAllowed { .. }
        | MemberError::UnknownAccount { .. } => Kind::Forbidden,
        MemberError::AlreadyMember { .. } | MemberError::TagHeld { .. } => Kind::Conflict,
        MemberError::NoFreeTag => Kind::NoFreeTag,
        MemberError::NotFound { .. } => Kind::NotFound,
        MemberError::Database { .. } => Kind::Io,
        MemberError::NotAnInvitation { .. }
        | MemberError::NotAMemberId { .. }
        | MemberError::NotATicket { .. }
        | MemberError::EmptyRole
        | MemberError::NotATag { .. }
        | MemberError::TagFreed { .. }
        | MemberError::NotHeld { .. }
        | MemberError::Invalid { .. }
        | MemberError::WrongSpace(_) => Kind::Validation,
    }
}
