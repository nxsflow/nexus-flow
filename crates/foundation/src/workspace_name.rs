//! **A workspace's name** (nxf 6j6v.q32p) — `<owner>/<repo>`, the part of an address from another
//! workspace that names the workspace: `nxsflow/nexus-flow/pm` is the persona `pm` of the
//! workspace named `nxsflow/nexus-flow`.
//!
//! Until this existed a workspace had no name anybody could write into a declaration: its origin is
//! the replica prefix, four random characters per clone that sync may reassign, and the repository
//! slug `nxs sync bind` derives from the git origin was hashed into the stream id and thrown away.
//!
//! The name is STORED, in `config.toml` under `[workspace] name`, so that it does not change when
//! the remote is renamed or the clone moves. Where nothing is stored it is DERIVED from
//! `git remote get-url origin` — the same source the stream id comes from, with the host dropped —
//! and [`ensure_name`] stores what it derived. A workspace without a git origin gets its name set
//! explicitly ([`set_name`], `nxs name <owner>/<repo>`).
//!
//! Every clone of one repository carries the same name. Which clone on this machine a name means is
//! the service registry's question, not this module's.

use std::path::Path;

use crate::error::{NxfError, Result};
use crate::workspace::{set_product_key, Workspace};

/// The `config.toml` section the name is stored in.
pub const SECTION: &str = "workspace";
/// The key under [`SECTION`].
pub const KEY: &str = "name";

/// Reduce any git remote spelling to `host/owner/repo` (lowercased, no scheme, no
/// credentials, no port, no `.git`). `None` when the input carries no usable path.
///
/// Moved here from `nxs sync`'s stream-id derivation (nxf 6j6v.q32p) so the stream id and the
/// workspace name are cut from one parse of the remote and cannot disagree about it.
pub fn normalize_remote(url: &str) -> Option<String> {
    let s = url.trim();
    // Whether a scheme was actually present is load-bearing below: git's scp-like syntax
    // `[user@]host:path` has NO port syntax at all, so a colon there is always the path
    // separator. A port is only grammatically possible once a scheme introduced `host:port`.
    // Do not fold this back into a bare "is the segment all-digits?" guess — that collides
    // scp remotes whose owner segment happens to be numeric (`host:1234/repo` and
    // `host:5678/repo` would both normalize to `host/repo`), which is exactly the property
    // this derivation exists to avoid.
    let had_scheme = s.contains("://");
    // Scheme, if any: https:// | ssh:// | git://
    let s = match s.find("://") {
        Some(i) => &s[i + 3..],
        None => s,
    };
    // Credentials: `git@host…`, `user:token@host…` — but ONLY an `@` occurring BEFORE the first
    // `/` is a credential separator (finding 6, final review). The old code stripped up to the
    // FIRST `@` anywhere in the string, including inside the path: `https://host/a@b/repo` was
    // treated as if `host/a` were credentials and normalized to `b/repo`, silently dropping the
    // host — the same defect class as the port heuristic just below (a bare positional guess
    // that ignores where in the URL's grammar the character actually sits), and it defeats the
    // whole point of host-scoping the derived `stream_id` (§4.1): two unrelated repos whose
    // paths happen to share an `x@y/…` segment would collide onto one stream.
    let s = {
        let path_start = s.find('/').unwrap_or(s.len());
        match s[..path_start].find('@') {
            Some(i) => &s[i + 1..],
            None => s,
        }
    };
    // A `:` before the first `/` is a port ONLY when a scheme was present (`ssh://host:22/…`);
    // in scp syntax (`host:owner/repo`) it is always the path separator, regardless of digits.
    let s = match s.find(':') {
        Some(colon) => {
            let rest = &s[colon + 1..];
            let end = rest.find('/').unwrap_or(rest.len());
            let seg = &rest[..end];
            let is_port = had_scheme && !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit());
            if is_port {
                format!("{}{}", &s[..colon], &rest[end..])
            } else {
                format!("{}/{}", &s[..colon], rest)
            }
        }
        None => s.to_string(),
    };
    let s = s.trim_end_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s);
    let s = s.trim_end_matches('/').to_ascii_lowercase();
    if !s.contains('/') || s.split('/').any(str::is_empty) {
        return None;
    }
    Some(s)
}

/// `git remote get-url origin` in `dir`. `None` when git is absent, the directory is not a
/// repo, or there is no `origin`.
pub fn origin_remote(dir: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8(out.stdout).ok()?;
    let url = url.trim().to_string();
    (!url.is_empty()).then_some(url)
}

/// The workspace name a remote implies: [`normalize_remote`] without its host —
/// `git@github.com:nxsflow/nexus-flow.git` is `nxsflow/nexus-flow`. A nested group keeps every
/// segment (`gitlab.com/group/sub/repo` is `group/sub/repo`). `None` for a remote with fewer than
/// two path segments.
pub fn name_from_remote(url: &str) -> Option<String> {
    let slug = normalize_remote(url)?;
    let (_host, path) = slug.split_once('/')?;
    is_valid_name(path).then(|| path.to_string())
}

/// Whether `name` can be a workspace name: at least two non-empty `/`-separated segments, each of
/// lowercase ASCII letters, digits, `.`, `_` and `-`, none of them `.` or `..`. Lowercase because a
/// derived name is lowercased, and a name written by hand must compare equal to it.
pub fn is_valid_name(name: &str) -> bool {
    let segments: Vec<&str> = name.split('/').collect();
    segments.len() >= 2
        && segments.iter().all(|s| {
            !s.is_empty()
                && *s != "."
                && *s != ".."
                && s.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
                })
        })
}

/// The name stored in this workspace's `config.toml`, if any.
pub fn stored_name(ws: &Workspace) -> Option<String> {
    ws.config
        .products
        .get(SECTION)
        .and_then(|s| s.get(KEY))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// The directory the workspace's repository lives in — the parent of `.nxs/`, read through the
/// symlink a workspace's `.nxs` may be, never resolved past it.
fn repo_root(ws: &Workspace) -> Option<&Path> {
    ws.dir.parent()
}

/// This workspace's name: the stored one, else the one its git origin implies. Writes nothing —
/// reading another workspace's name (the service registry does, for every workspace it knows)
/// must not change that workspace. [`ensure_name`] is the writing form.
pub fn name_of(ws: &Workspace) -> Option<String> {
    stored_name(ws).or_else(|| {
        repo_root(ws)
            .and_then(origin_remote)
            .and_then(|url| name_from_remote(&url))
    })
}

/// [`name_of`], storing a derived name so it stays put from then on. `None` for a workspace with
/// neither a stored name nor a git origin — it needs `nxs name <owner>/<repo>`.
pub fn ensure_name(ws: &Workspace) -> Result<Option<String>> {
    if let Some(name) = stored_name(ws) {
        return Ok(Some(name));
    }
    let Some(name) = name_of(ws) else {
        return Ok(None);
    };
    set_name(&ws.dir, &name)?;
    Ok(Some(name))
}

/// Store `name` as the workspace's name. `dir` is the workspace directory (where `config.toml`
/// lives). Refuses a name [`is_valid_name`] rejects, saying what a name looks like. Returns whether
/// the file changed.
pub fn set_name(dir: &Path, name: &str) -> Result<bool> {
    if !is_valid_name(name) {
        return Err(NxfError::validation(format!(
            "`{name}` is not a workspace name — it is `<owner>/<repo>`: at least two segments of \
             lowercase letters, digits, `.`, `_` and `-`, separated by `/`"
        )));
    }
    set_product_key(dir, SECTION, KEY, toml::Value::String(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_names_its_workspace_without_the_host() {
        for url in [
            "git@github.com:nxsflow/nexus-flow.git",
            "https://github.com/nxsflow/nexus-flow",
            "https://user:token@GitHub.com/NxsFlow/Nexus-Flow.git/",
            "ssh://git@github.com:22/nxsflow/nexus-flow.git",
        ] {
            assert_eq!(
                name_from_remote(url).as_deref(),
                Some("nxsflow/nexus-flow"),
                "{url}"
            );
        }
        assert_eq!(
            name_from_remote("git@gitlab.com:group/sub/repo.git").as_deref(),
            Some("group/sub/repo")
        );
        assert_eq!(name_from_remote("https://example.com/only-one"), None);
    }

    #[test]
    fn a_name_has_two_segments_of_a_narrow_charset() {
        assert!(is_valid_name("nxsflow/nexus-flow"));
        assert!(is_valid_name("group/sub/repo"));
        for bad in [
            "",
            "pm",
            "nxsflow/",
            "/nexus-flow",
            "NxsFlow/nexus-flow",
            "a/../b",
            "a/b c",
        ] {
            assert!(!is_valid_name(bad), "{bad:?}");
        }
    }
}
