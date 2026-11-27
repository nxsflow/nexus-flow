//! The NoCentralDB role each nexus-flow role gets in each kind of space (RFC 0001, "Roles in a
//! space"):
//!
//! | Role | Repository space | Coordination space |
//! | --- | --- | --- |
//! | `owner` (the owner's devices) | admin | admin |
//! | `developer` (other developers' devices) | writer | writer |
//! | `pm` | admin | writer |
//! | `app` (manufakt.io) | writer | writer |
//! | `migration` | writer | not a member |
//! | a role session (`coder`, `legal`, …) | writer | not a member |
//!
//! A developer's device the owner makes an admin is raised afterwards; `nxs member add` adds no
//! one to an ad-hoc space, where pm adds its sessions with NoCentralDB alone (RFC 0009, "Members
//! and roles").

use crate::space_kind::SpaceKind;

/// A member's role in one space (NoCentralDB RFC 0008, "Roles and rights"). nexus-flow adds no
/// readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceRole {
    /// Writes content.
    Writer,
    /// Also adds and removes members.
    Admin,
}

impl SpaceRole {
    pub(crate) fn sdk(self) -> nocentraldb::Role {
        match self {
            SpaceRole::Writer => nocentraldb::Role::Writer,
            SpaceRole::Admin => nocentraldb::Role::Admin,
        }
    }
}

/// The role a member of nexus-flow role `role` gets in a space of kind `kind`; `None` when it is
/// not added there.
pub fn space_role(role: &str, kind: SpaceKind) -> Option<SpaceRole> {
    use SpaceRole::{Admin, Writer};
    match (kind, role) {
        (SpaceKind::AdHoc, _) => None,
        (_, "owner") => Some(Admin),
        (SpaceKind::Repository, "pm") => Some(Admin),
        (_, "developer" | "pm" | "app") => Some(Writer),
        (SpaceKind::Repository, _) => Some(Writer),
        (SpaceKind::Coordination, _) => None,
    }
}
