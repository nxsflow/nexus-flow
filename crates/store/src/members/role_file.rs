//! A session's role names a role file (RFC 0006, "The factory's files"): the role `coder` is the
//! file `factory/roles/coder.md` of the working tree. The frame's own roles (RFC 0001, "The role
//! record") name no file.

use std::path::{Path, PathBuf};

use super::MemberError;

/// The roles of the frame itself, which name no role file.
const FRAME_ROLES: [&str; 4] = ["owner", "developer", "app", "migration"];

/// Whether `role` is one of the frame's own roles.
pub(super) fn is_frame_role(role: &str) -> bool {
    FRAME_ROLES.contains(&role)
}

/// The role file of the session role `role` in the working tree at `root`; refused when it is
/// missing.
pub(super) fn role_file(root: &Path, role: &str) -> Result<PathBuf, MemberError> {
    let path = root.join("factory").join("roles").join(format!("{role}.md"));
    if path.is_file() {
        Ok(path)
    } else {
        Err(MemberError::NoRoleFile {
            role: role.to_owned(),
            path,
        })
    }
}
