//! The NoCentralDB role a member gets in each kind of space, from its nexus-flow role (RFC 0001,
//! "Roles in a space"). A role session, such as `coder` or `legal`, is a writer of the repository
//! space and no member of the coordination space; the migration member likewise.

use crate::members::{space_role, SpaceRole};
use crate::space_kind::SpaceKind;

#[test]
fn the_roles_in_a_repository_space_follow_rfc_0001() {
    let table = [
        ("owner", SpaceRole::Admin),
        ("developer", SpaceRole::Writer),
        ("pm", SpaceRole::Admin),
        ("app", SpaceRole::Writer),
        ("migration", SpaceRole::Writer),
        ("coder", SpaceRole::Writer),
        ("legal", SpaceRole::Writer),
    ];
    for (role, expected) in table {
        assert_eq!(
            space_role(role, SpaceKind::Repository),
            Some(expected),
            "{role}"
        );
    }
}

#[test]
fn the_roles_in_a_coordination_space_follow_rfc_0001() {
    let table = [
        ("owner", Some(SpaceRole::Admin)),
        ("developer", Some(SpaceRole::Writer)),
        ("pm", Some(SpaceRole::Writer)),
        ("app", Some(SpaceRole::Writer)),
        ("migration", None),
        ("coder", None),
        ("legal", None),
    ];
    for (role, expected) in table {
        assert_eq!(
            space_role(role, SpaceKind::Coordination),
            expected,
            "{role}"
        );
    }
}

#[test]
fn nxs_member_add_adds_no_one_to_an_ad_hoc_space() {
    for role in ["owner", "developer", "pm", "app", "migration", "coder"] {
        assert_eq!(space_role(role, SpaceKind::AdHoc), None, "{role}");
    }
}
