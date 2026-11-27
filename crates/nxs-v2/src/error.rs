//! Errors as v1 prints them (RFC 0001, "Reading and writing"; RFC 0002, "JSON"): the error object
//! `{"error":{"kind":"<code>","msg":"<message>"}}` on stdout with exit code 1, where `kind` is a
//! stable code an agent matches on and `msg` is for a person. With `--md`, `error: <msg>` goes to
//! stderr instead, as v1's human output does. The codes are v1's, `no_key_store` (NEX-224) and
//! `timeout` (NEX-102); the member commands use v1's `forbidden` and `not_found` (NEX-99), and
//! `no_free_tag` from RFC 0002 (NEX-108); a retired v1 entry point is `retired_entry_point`
//! (RFC 0008, NEX-191); `nxs decision add` refuses with
//! `validation`, or `not_found` for a decision to supersede that the local copy does not hold.

use nexus_flow_seam::merge_gate::GitError;
use nexus_flow_seam::{
    ConfigError, DataDirError, DecisionError, Error, InitError, JoinError, MemberError, Problem,
    RootError, SyncError,
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
    /// `nxs` was started as `nxf`, `nxm` or `nxc`, which v2 retires (RFC 0008, NEX-191).
    RetiredEntryPoint,
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
            Kind::RetiredEntryPoint => "retired_entry_point",
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
            // nxs opens a repository, which always has a checkout.
            Error::NoCheckout => Kind::Validation,
            Error::Install(_) => Kind::Io,
            Error::Member(member) => member_kind(member),
            Error::Decision(decision) => decision_kind(decision),
            Error::Coordination(join) => join_kind(join),
            Error::Git(GitError::NotACommit { .. }) => Kind::Validation,
            Error::Git(GitError::Failed { .. }) => Kind::Io,
        };
        CliError { kind, msg }
    }
}

fn open_kind(e: &InitError) -> Kind {
    match e {
        InitError::Root(root) => root_kind(root),
        InitError::Config(config) => config_kind(config),
        InitError::DataDir { .. } => Kind::Io,
        InitError::SpaceId { .. } => Kind::Validation,
        InitError::KeyStoreUnavailable => Kind::NoKeyStore,
        InitError::Settings(_) => Kind::Validation,
        InitError::Database { .. } => Kind::Io,
        // NXS_SESSION names a session that is not running on this device: a setting nxs refuses.
        InitError::NoSession { .. } => Kind::Validation,
    }
}

fn root_kind(e: &RootError) -> Kind {
    match e {
        RootError::NotFound { .. } => Kind::NoWorkspace,
        RootError::DataDirInside { .. } => Kind::Validation,
    }
}

fn config_kind(e: &ConfigError) -> Kind {
    match e.problem {
        Problem::AlreadyNamesSpace { .. } | Problem::AlreadyExists => Kind::Conflict,
        Problem::Io { .. } => Kind::Io,
        _ => Kind::Validation,
    }
}

/// A second coordination space is `conflict`: the file already names one, and nxs does not
/// overwrite it (NEX-104).
fn join_kind(e: &JoinError) -> Kind {
    match e {
        JoinError::Root(root) => root_kind(root),
        JoinError::Config(config) => config_kind(config),
        JoinError::OtherCoordination { .. } => Kind::Conflict,
        JoinError::OwnSpace { .. } | JoinError::CannotPlace { .. } => Kind::Validation,
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

fn decision_kind(e: &DecisionError) -> Kind {
    match e {
        DecisionError::SupersedesUnknown { .. } => Kind::NotFound,
        DecisionError::Database { .. } => Kind::Io,
        _ => Kind::Validation,
    }
}
