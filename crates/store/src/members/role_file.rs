//! A session's role names a role file (RFC 0006, "The factory's files"): the role `coder` is the
//! file `factory/roles/coder.md` of the working tree. The frame's own roles (RFC 0001, "The role
//! record") name no file. A role's name matches `^[a-z][a-z0-9-]{0,62}$`, so it never reaches
//! outside `factory/roles/`.

use std::path::{Path, PathBuf};

use super::MemberError;

/// The roles of the frame itself, which name no role file.
const FRAME_ROLES: [&str; 4] = ["owner", "developer", "app", "migration"];

/// The role file of `role` in the working tree at `root`: `None` for a role of the frame, even
/// when a file of its name exists, and refused when a session's role is no role name or has none.
pub(super) fn role_file(root: &Path, role: &str) -> Result<Option<PathBuf>, MemberError> {
    if FRAME_ROLES.contains(&role) {
        return Ok(None);
    }
    if !is_role_name(role) {
        return Err(MemberError::NotARoleName {
            role: role.to_owned(),
        });
    }
    let path = root.join("factory").join("roles").join(format!("{role}.md"));
    if path.is_file() {
        Ok(Some(path))
    } else {
        Err(MemberError::NoRoleFile {
            role: role.to_owned(),
            path,
        })
    }
}

/// Whether `role` matches `^[a-z][a-z0-9-]{0,62}$`.
fn is_role_name(role: &str) -> bool {
    let mut bytes = role.bytes();
    let first = bytes.next().is_some_and(|b| b.is_ascii_lowercase());
    first
        && role.len() <= 63
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
