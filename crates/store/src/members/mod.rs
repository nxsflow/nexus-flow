//! The members of a repository's spaces (RFC 0001, "Members", "The role record", "Roles in a
//! space"): adding one from its invitation with its `spaceMember` record, removing one, and
//! listing the current members with their records.
//!
//! **Adding.** `add` adds the invitation's member to every open space its role belongs in, as
//! "Roles in a space" gives it ([`space_role`]): the repository space, and the coordination space
//! for every role but a role session's and the migration member's. In each it adds the member and
//! then writes its `spaceMember` record, or rewrites the one a member added before still has. The
//! record carries the member's ticket tag (RFC 0002, "Ticket ids"), the same in every space: the
//! one `--tag` names, or a free one ([`taken`] says which). It checks first that this member is an
//! admin of each of those spaces, that each is held, that the new member is in none of them, and
//! that the tag can be given, so a refusal changes nothing.
//!
//! **Removing.** `remove` removes a member from every open space it is a member of, after the same
//! checks. Its record stays: who wrote what stays readable after its member is gone (RFC 0001,
//! "Who decided").
//!
//! **Listing.** `list` gives the repository space's current members in member-id order, each with
//! its kind and account from its certificate and its role, ticket and tag from its record, and
//! whether another current member holds the same tag. A member nexus-flow did not add, such as the
//! device that created the space, has no record: its role, ticket and tag are `None`.

use std::fmt;
use std::str::FromStr;

use nocentraldb::{
    AddMember, Db, Error as SdkError, ErrorCode, Invitation, MemberId, MemberInfo, ObjectId, Space,
};

use crate::id::SpaceId;
use crate::opened::Opened;
use crate::space_kind::WrongSpace;
use crate::write::WriteError;

pub(crate) mod record;
mod retag;
mod roles;
pub mod tags;
mod taken;

pub use retag::retag;
pub use roles::{space_role, SpaceRole};

use tags::Tag;
use taken::Taken;

/// What a member is, from its certificate (NoCentralDB's `MemberKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemberKind {
    /// A person's device.
    Device,
    /// An agent, such as pm, paired into a person's account.
    Agent,
    /// A sub-agent, such as a role session, certified by its parent agent.
    SubAgent,
}

impl MemberKind {
    /// The kind as `nxs member list` prints it.
    pub fn as_str(self) -> &'static str {
        match self {
            MemberKind::Device => "device",
            MemberKind::Agent => "agent",
            MemberKind::SubAgent => "subAgent",
        }
    }
}

/// A reference to the ticket a session was started on: the space and the object id of its
/// target, written `<space id>/<object id>`, as RFC 0002 writes the target of a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TicketRef {
    pub space: SpaceId,
    pub object: ObjectId,
}

impl fmt::Display for TicketRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.space, self.object)
    }
}

impl FromStr for TicketRef {
    type Err = MemberError;

    fn from_str(text: &str) -> Result<Self, MemberError> {
        let not = || MemberError::NotATicket {
            text: text.to_owned(),
        };
        let (space, object) = text.split_once('/').ok_or_else(not)?;
        Ok(TicketRef {
            space: space.parse().map_err(|_| not())?,
            object: object.parse().map_err(|_| not())?,
        })
    }
}

/// One current member of a space, with its role record (RFC 0001, "Public surfaces this RFC
/// brings", `nxs member list`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The NoCentralDB member id, as 64 lowercase hex digits.
    pub member: String,
    /// Whether it is a device, an agent or a sub-agent.
    pub kind: MemberKind,
    /// The role its `spaceMember` record gives it; `None` when it has none.
    pub role: Option<String>,
    /// The ticket its record names, for a session started on one.
    pub ticket: Option<TicketRef>,
    /// The member that signed a sub-agent's certificate; `None` for a device or an agent.
    pub parent: Option<String>,
    /// Its ticket tag, such as `ka`, as its record holds it; `None` when it has none.
    pub tag: Option<String>,
    /// Whether another current member holds the same tag, as two inviters picking offline at
    /// once can leave it (RFC 0002, "Two members with one tag").
    pub tag_shared: bool,
    /// The fingerprint of the account its certificate chain leads to: twelve blocks of five
    /// digits, as a person compares it.
    pub account: String,
}

/// Why a member could not be added, removed or listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberError {
    /// The text is not an invitation.
    NotAnInvitation { message: String },
    /// The text is not a member id.
    NotAMemberId { text: String },
    /// The text is not a reference to a ticket.
    NotATicket { text: String },
    /// The role is empty.
    EmptyRole,
    /// The text is not a tag.
    NotATag { text: String },
    /// A current member holds the tag.
    TagHeld { tag: String },
    /// Only removed members held the tag, and nxs does not hand a freed tag out again yet.
    TagFreed { tag: String },
    /// Every tag is held.
    NoFreeTag,
    /// A space the change needs is named but not held yet: this database has not synced it.
    NotHeld { space: SpaceId },
    /// This member is not an admin of a space the change needs.
    NotAdmin { space: SpaceId },
    /// The new member is already a member of the repository space.
    AlreadyMember { space: SpaceId },
    /// The member is a member of none of the open spaces.
    NotFound { member: String },
    /// NoCentralDB does not let this member do it, such as a device adding a sub-agent.
    NotAllowed { message: String },
    /// The invitation's account is new to this device, and a person has not compared its
    /// fingerprint.
    UnknownAccount { message: String },
    /// NoCentralDB refused a value, such as a certificate that does not hold or this member
    /// removing itself.
    Invalid { message: String },
    /// The record may not be written into this space.
    WrongSpace(WrongSpace),
    /// The database failed.
    Database { message: String },
}

impl fmt::Display for MemberError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemberError::NotAnInvitation { message } => write!(f, "not an invitation: {message}"),
            MemberError::NotAMemberId { text } => {
                write!(
                    f,
                    "{text:?} is not a member id: one is 64 lowercase hex digits"
                )
            }
            MemberError::NotATicket { text } => write!(
                f,
                "{text:?} is not a reference to a ticket: one is <space id>/<object id>"
            ),
            MemberError::EmptyRole => f.write_str("the role is empty"),
            MemberError::NotATag { text } => write!(
                f,
                "{text:?} is not a tag: one is two characters matching ^[a-z][a-z0-9]$"
            ),
            MemberError::TagHeld { tag } => {
                write!(f, "the tag {tag} is held by a current member of the space")
            }
            MemberError::TagFreed { tag } => write!(
                f,
                "the tag {tag} was held by a removed member, and nxs does not hand a freed tag out again yet"
            ),
            MemberError::NoFreeTag => f.write_str(
                "every one of the 936 tags is in use; remove a member to free one",
            ),
            MemberError::NotHeld { space } => write!(
                f,
                "this device does not hold the space {space} yet; sync it with nxs sync first"
            ),
            MemberError::NotAdmin { space } => write!(
                f,
                "only an admin of the space {space} adds or removes its members"
            ),
            MemberError::AlreadyMember { space } => {
                write!(
                    f,
                    "the invited member is already a member of the space {space}"
                )
            }
            MemberError::NotFound { member } => {
                write!(f, "{member} is not a member of this repository's spaces")
            }
            MemberError::NotAllowed { message }
            | MemberError::UnknownAccount { message }
            | MemberError::Invalid { message }
            | MemberError::Database { message } => f.write_str(message),
            MemberError::WrongSpace(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for MemberError {}

impl From<WriteError> for MemberError {
    fn from(e: WriteError) -> Self {
        match e {
            WriteError::WrongSpace(e) => MemberError::WrongSpace(e),
            WriteError::NotOpen(space) | WriteError::NotHeld(space) => {
                MemberError::NotHeld { space }
            }
            WriteError::WriteOnce(e) => MemberError::Invalid {
                message: e.to_string(),
            },
            WriteError::MoveOnly(e) => MemberError::Invalid {
                message: e.to_string(),
            },
            WriteError::Database(e) => e.into(),
        }
    }
}

impl From<SdkError> for MemberError {
    fn from(e: SdkError) -> Self {
        let message = e.to_string();
        match e.code() {
            ErrorCode::NotAllowed | ErrorCode::NotMember => MemberError::NotAllowed { message },
            ErrorCode::UnknownAccount => MemberError::UnknownAccount { message },
            ErrorCode::InvalidValue | ErrorCode::InvalidCertificate | ErrorCode::LastAdmin => {
                MemberError::Invalid { message }
            }
            _ => MemberError::Database { message },
        }
    }
}

/// Adds the member of `invitation` to the open spaces its `role` belongs in, each with its
/// `spaceMember` record naming `role`, `ticket` and its tag, `tag` or a free one, and returns it
/// as the repository space lists it.
pub async fn add(
    opened: &Opened,
    invitation: &str,
    role: &str,
    ticket: Option<&str>,
    tag: Option<&str>,
) -> Result<Member, MemberError> {
    let parsed: Invitation =
        invitation
            .trim()
            .parse()
            .map_err(|e: SdkError| MemberError::NotAnInvitation {
                message: e.to_string(),
            })?;
    let role = role.trim();
    if role.is_empty() {
        return Err(MemberError::EmptyRole);
    }
    let ticket = ticket.map(str::parse::<TicketRef>).transpose()?;
    let named = tag.map(parse_tag).transpose()?;
    let targets = targets(opened, role)?;
    for (_, space, _) in &targets {
        admin(space).await?;
    }
    let taken = Taken::read(opened.db(), targets.iter().map(|(_, space, _)| *space)).await?;
    match named {
        Some(tag) => taken.check(tag)?,
        None => taken.any_free()?,
    }
    // An invitation does not show its member's id, so it is read from the first space once the
    // member is added there. A member already in the repository space is refused before anything
    // is written; one a coordination admin already added there keeps that membership and gets its
    // record.
    let mut added: Option<(MemberId, Tag)> = None;
    for (id, space, space_role) in &targets {
        let before = ids(space).await?;
        let options = AddMember {
            role: space_role.sdk(),
            fingerprint: None,
        };
        let (member, tag) = match (space.members().add(&parsed, options).await, added) {
            (Ok(_), Some(added)) => added,
            (Ok(_), None) => {
                let member = new_member(space, &before).await?;
                let own = record::read(opened.db(), space, member).await?;
                let own = own.and_then(|r| r.tag);
                (member, taken.choose(named, own.as_deref(), member)?)
            }
            (Err(e), Some(added)) if e.code() == ErrorCode::AlreadyMember => added,
            (Err(e), None) if e.code() == ErrorCode::AlreadyMember => {
                return Err(MemberError::AlreadyMember { space: *id })
            }
            (Err(e), _) => return Err(e.into()),
        };
        record::write(opened, id, space, member, role, ticket.as_ref(), Some(&tag)).await?;
        added = Some((member, tag));
    }
    let (member, _) = added.expect("the repository space is always a target");
    let (_, repository, _) = &targets[0];
    listed(opened.db(), repository, member).await
}

/// Removes `member` from every open space it is a member of, and returns it as the first of
/// them listed it. Its `spaceMember` records stay.
pub async fn remove(opened: &Opened, member: &str) -> Result<Member, MemberError> {
    let (id, spaces) = spaces_of(opened, member).await?;
    let (_, first) = spaces[0];
    let removed = listed(opened.db(), first, id).await?;
    for (_, space) in &spaces {
        space.members().remove(id).await?;
    }
    Ok(removed)
}

/// The member id `member` and the open spaces it is a current member of, at least one, after
/// checking that this member is an admin of each.
pub(crate) async fn spaces_of<'a>(
    opened: &'a Opened,
    member: &str,
) -> Result<(MemberId, Vec<(SpaceId, &'a Space)>), MemberError> {
    let id: MemberId = member.parse().map_err(|_| MemberError::NotAMemberId {
        text: member.to_owned(),
    })?;
    let mut spaces = Vec::new();
    for (space_id, space) in opened.spaces() {
        let Some(space) = space else { continue };
        if ids(space).await?.contains(&id) {
            spaces.push((*space_id, space));
        }
    }
    if spaces.is_empty() {
        return Err(MemberError::NotFound {
            member: member.to_owned(),
        });
    }
    for (_, space) in &spaces {
        admin(space).await?;
    }
    Ok((id, spaces))
}

/// The current members of the repository space, in member-id order, with their records. A
/// repository space the database does not hold yet has none.
pub async fn list(opened: &Opened) -> Result<Vec<Member>, MemberError> {
    match opened.spaces().first() {
        Some((_, Some(space))) => list_in(opened.db(), space).await,
        _ => Ok(Vec::new()),
    }
}

/// The current members of `space`, in member-id order, with their records.
pub(crate) async fn list_in(db: &Db, space: &Space) -> Result<Vec<Member>, MemberError> {
    let mut members = Vec::new();
    for info in space.members().list().await? {
        members.push(member(db, space, &info).await?);
    }
    let holders = |tag: &str| {
        members
            .iter()
            .filter(|m| m.tag.as_deref() == Some(tag))
            .count()
    };
    let shared: Vec<bool> = members
        .iter()
        .map(|m| m.tag.as_deref().is_some_and(|tag| holders(tag) > 1))
        .collect();
    for (member, shared) in members.iter_mut().zip(shared) {
        member.tag_shared = shared;
    }
    Ok(members)
}

/// `text` as a tag.
pub(crate) fn parse_tag(text: &str) -> Result<Tag, MemberError> {
    text.parse().map_err(|_| MemberError::NotATag {
        text: text.to_owned(),
    })
}

/// The spaces `role` belongs in, among the open ones, each with the role it gets there: the
/// repository space first, as `Opened` holds it. A space it belongs in that is not held is
/// refused.
fn targets<'a>(
    opened: &'a Opened,
    role: &str,
) -> Result<Vec<(SpaceId, &'a Space, SpaceRole)>, MemberError> {
    let mut targets = Vec::new();
    for (id, space) in opened.spaces() {
        let Some(kind) = opened.kind(id) else {
            continue;
        };
        let Some(space_role) = space_role(role, kind) else {
            continue;
        };
        match space {
            Some(space) => targets.push((*id, space, space_role)),
            None => return Err(MemberError::NotHeld { space: *id }),
        }
    }
    if targets.is_empty() {
        let space = opened.spaces().first().map(|(id, _)| *id);
        return Err(match space {
            Some(space) => MemberError::NotHeld { space },
            None => MemberError::Database {
                message: "no repository space is open".to_owned(),
            },
        });
    }
    Ok(targets)
}

/// Refuses unless this database's member is an admin of `space`.
async fn admin(space: &Space) -> Result<(), MemberError> {
    let me = space
        .members()
        .list()
        .await?
        .into_iter()
        .find(|m| m.is_self);
    match me.and_then(|m| m.role) {
        Some(nocentraldb::Role::Admin) => Ok(()),
        _ => Err(MemberError::NotAdmin {
            space: SpaceId(space.id()),
        }),
    }
}

/// The ids of the current members of `space`.
pub(crate) async fn ids(space: &Space) -> Result<Vec<MemberId>, MemberError> {
    Ok(space
        .members()
        .list()
        .await?
        .into_iter()
        .map(|m| m.id)
        .collect())
}

/// The one member of `space` that is not among `before`.
async fn new_member(space: &Space, before: &[MemberId]) -> Result<MemberId, MemberError> {
    ids(space)
        .await?
        .into_iter()
        .find(|id| !before.contains(id))
        .ok_or_else(|| MemberError::Database {
            message: "the added member is not listed".to_owned(),
        })
}

/// `member` as `space` lists it, with its record.
async fn listed(db: &Db, space: &Space, member: MemberId) -> Result<Member, MemberError> {
    let member = member.to_string();
    list_in(db, space)
        .await?
        .into_iter()
        .find(|m| m.member == member)
        .ok_or(MemberError::NotFound { member })
}

/// The member `info` describes, with its record in `space`.
async fn member(db: &Db, space: &Space, info: &MemberInfo) -> Result<Member, MemberError> {
    let record = record::read(db, space, info.id).await?;
    Ok(Member {
        member: info.id.to_string(),
        kind: kind(info.kind),
        role: record.as_ref().and_then(|r| r.role.clone()),
        ticket: record.as_ref().and_then(|r| r.ticket),
        parent: None,
        tag: record.and_then(|r| r.tag),
        tag_shared: false,
        account: info.account.to_string(),
    })
}

fn kind(kind: nocentraldb::MemberKind) -> MemberKind {
    match kind {
        nocentraldb::MemberKind::Agent => MemberKind::Agent,
        nocentraldb::MemberKind::SubAgent => MemberKind::SubAgent,
        _ => MemberKind::Device,
    }
}

#[cfg(test)]
mod tests;
