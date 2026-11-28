//! A repository's name (RFC 0009, "Repositories in the space"): the one a caller gives, or by
//! default the last path segment of the git remote `origin`, without `.git`.

use std::path::Path;
use std::process::Command;

use super::CoordinationError;

/// `text`, when it is a repository name: not empty, and without a space or a control character.
pub(crate) fn check(text: &str) -> Result<String, CoordinationError> {
    if text.is_empty() || text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(CoordinationError::NotAName {
            text: text.to_owned(),
        });
    }
    Ok(text.to_owned())
}

/// The name the git remote `origin` of the checkout at `root` gives; [`CoordinationError::NoName`]
/// when there is no such remote, or its url ends in no name.
pub(crate) fn from_origin(root: &Path) -> Result<String, CoordinationError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|_| CoordinationError::NoName)?;
    if !output.status.success() {
        return Err(CoordinationError::NoName);
    }
    let url = String::from_utf8_lossy(&output.stdout);
    last_segment(url.trim()).ok_or(CoordinationError::NoName)
}

/// The last path segment of `url`, after its last `/` or `:`, without a trailing `/` or `.git`.
fn last_segment(url: &str) -> Option<String> {
    let url = url.trim_end_matches('/');
    let segment = url.rsplit(['/', ':']).next()?;
    let segment = segment.strip_suffix(".git").unwrap_or(segment);
    check(segment).ok()
}
