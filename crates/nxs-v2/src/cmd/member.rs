//! `nxs member add <invitation> --role <role> [--ticket <ref>]`, `nxs member remove <member>` and
//! `nxs member list` (RFC 0001, "Members", "The role record", "Public surfaces this RFC brings"),
//! through the seam's `Engine::member_add`, `member_remove` and `members`.
//!
//! `list` prints every current member of the repository space, in member-id order, as an object
//! with `member`, `kind`, `role`, `ticket`, `parent` and `account`; `add` and `remove` print the
//! member they added or removed as the same object. A ticket is written `<space id>/<object id>`.

use std::path::Path;

use clap::{Args, Subcommand};
use nexus_flow_seam::{DataDirError, Engine, Member, Options};
use serde::Serialize;

use crate::block_on::block_on;
use crate::error::CliError;
use crate::output::Render;

/// The verbs of `nxs member`.
#[derive(Debug, Subcommand)]
pub enum MemberCommand {
    /// Add a member from its invitation, with its role record.
    Add(AddArgs),
    /// Remove a member, by its id, from the repository's spaces.
    Remove(RemoveArgs),
    /// List the repository space's members with their role records.
    List,
}

/// The arguments of `nxs member add`.
#[derive(Debug, Args)]
pub struct AddArgs {
    /// The invitation the new member printed (`ncdb-invite-…`).
    invitation: String,
    /// The member's role: owner, developer, pm, app, migration, or a session's role.
    #[arg(long)]
    role: String,
    /// The ticket a session was started on, as <space id>/<object id>.
    #[arg(long, value_name = "ref")]
    ticket: Option<String>,
}

/// The arguments of `nxs member remove`.
#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// The member's id, 64 hex digits, as `nxs member list` prints it.
    member: String,
}

/// A member as `nxs member` prints it.
#[derive(Debug, Serialize)]
pub struct Row {
    member: String,
    kind: &'static str,
    role: Option<String>,
    ticket: Option<String>,
    parent: Option<String>,
    account: String,
}

/// What `nxs member` prints: one member for `add` and `remove`, every member for `list`.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Printed {
    One(Row),
    All(Vec<Row>),
}

/// Runs `nxs member <verb>` in `dir`.
pub fn run(
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
    command: MemberCommand,
) -> Result<Printed, CliError> {
    let engine = block_on(Engine::open_with(dir, options()?))?;
    Ok(match command {
        MemberCommand::Add(args) => Printed::One(
            block_on(engine.member_add(&args.invitation, &args.role, args.ticket.as_deref()))?
                .into(),
        ),
        MemberCommand::Remove(args) => {
            Printed::One(block_on(engine.member_remove(&args.member))?.into())
        }
        MemberCommand::List => Printed::All(
            block_on(engine.members())?
                .into_iter()
                .map(Row::from)
                .collect(),
        ),
    })
}

impl From<Member> for Row {
    fn from(member: Member) -> Self {
        Row {
            member: member.member,
            kind: member.kind.as_str(),
            role: member.role,
            ticket: member.ticket.map(|t| t.to_string()),
            parent: member.parent,
            account: member.account,
        }
    }
}

impl Render for Printed {
    fn markdown(&self) -> String {
        let rows = match self {
            Printed::One(row) => std::slice::from_ref(row),
            Printed::All(rows) => rows.as_slice(),
        };
        let mut text = String::from(
            "| member | kind | role | ticket | parent | account |\n| --- | --- | --- | --- | --- | --- |\n",
        );
        for row in rows {
            let cell = |value: &Option<String>| value.clone().unwrap_or_else(|| "-".to_owned());
            text.push_str(&format!(
                "| `{}` | {} | {} | {} | {} | {} |\n",
                row.member,
                row.kind,
                cell(&row.role),
                cell(&row.ticket),
                cell(&row.parent),
                row.account
            ));
        }
        text
    }
}
