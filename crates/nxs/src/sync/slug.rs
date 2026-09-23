//! Stream identity derived from the git remote (spec §4.1) and the `stream_id` charset
//! gate (§6). Pure functions — the only IO is [`origin_remote`], kept separate so the
//! derivation itself is table-testable.

use sha2::{Digest, Sha256};

/// The charset a `stream_id` must satisfy to survive a URL path unmangled.
///
/// Deliberately narrower than "whatever percent-encoding can carry": API Gateway and
/// CloudFront — the stack the deployed relay sits behind — normalize `%2F` in paths, so a
/// client-side encoding fix alone can be green locally against axum and still 404 once
/// deployed. Refusing the id at BIND time turns that into an error at the moment someone
/// can still fix it. Never applied on LOAD: that would break existing bindings.
pub fn is_valid_stream_id(id: &str) -> bool {
    // `.` and `..` pass the charset and are path segments a URL normalises away:
    // `{base}/streams/../ops` is `{base}/ops` (review of PR #487, Integrity #7).
    !id.is_empty()
        && id != "."
        && id != ".."
        && id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.' || b == b'-'
        })
}

/// Reduce any git remote spelling to `host/owner/repo` (lowercased, no scheme, no
/// credentials, no port, no `.git`). `None` when the input carries no usable path.
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

/// `stream-` + the first 24 hex chars of `sha256(host/owner/repo)`.
///
/// Path-safe by construction, which is why the literal slug from the upstream design note's
/// example is NOT used as the id. Same `hex[..24]` idiom as chat's DM channel ids.
pub fn stream_id_from_remote(url: &str) -> Option<String> {
    let slug = normalize_remote(url)?;
    let mut h = Sha256::new();
    h.update(slug.as_bytes());
    Some(format!("stream-{}", &format!("{:x}", h.finalize())[..24]))
}

/// `git remote get-url origin` in `dir`. `None` when git is absent, the directory is not a
/// repo, or there is no `origin` — the caller turns that into the error that names
/// `--create`/`--join`.
pub fn origin_remote(dir: &std::path::Path) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_remote_form_of_one_repo_normalizes_to_the_same_slug() {
        for url in [
            "git@github.com:nxsflow/manufakt-io.git",
            "https://github.com/nxsflow/manufakt-io.git",
            "https://github.com/nxsflow/manufakt-io",
            "ssh://git@github.com:22/nxsflow/manufakt-io.git",
            "https://user:token@github.com/nxsflow/manufakt-io.git",
            "  https://GitHub.com/NxsFlow/Manufakt-IO.git/  ",
        ] {
            assert_eq!(
                normalize_remote(url).as_deref(),
                Some("github.com/nxsflow/manufakt-io"),
                "remote form {url:?}",
            );
        }
    }

    #[test]
    fn nested_groups_survive_normalization() {
        assert_eq!(
            normalize_remote("git@gitlab.com:group/sub/repo.git").as_deref(),
            Some("gitlab.com/group/sub/repo"),
        );
    }

    #[test]
    fn scp_syntax_has_no_port_a_numeric_owner_is_a_path_segment_not_a_port() {
        // scp syntax has no port: a numeric owner segment is a PATH segment, not a port.
        assert_eq!(
            normalize_remote("git@host:1234/repo.git").as_deref(),
            Some("host/1234/repo"),
        );
        assert_ne!(
            stream_id_from_remote("git@host:1234/repo.git"),
            stream_id_from_remote("git@host:5678/repo.git"),
            "distinct numeric owners must not collide onto one stream",
        );
        // A scheme DOES permit a port, and it is still dropped.
        assert_eq!(
            normalize_remote("ssh://git@host:22/owner/repo.git").as_deref(),
            Some("host/owner/repo"),
        );
    }

    #[test]
    fn an_at_sign_in_the_path_is_not_mistaken_for_a_credential_separator() {
        // Finding 6 (final review): the OLD code stripped everything up to the FIRST `@`
        // anywhere in the string, including inside the PATH — `https://host/a@b/repo` was
        // treated as if `host/a` were credentials, normalizing to `b/repo` and dropping the
        // host entirely, which collides distinct repos and defeats the host-scoping §4.1
        // exists to provide. Only an `@` before the first `/` is a credential separator.
        assert_eq!(
            normalize_remote("https://host/a@b/repo").as_deref(),
            Some("host/a@b/repo"),
            "the host survives — the `@` here is part of the path, not credentials"
        );
        // The genuine credential case must still work exactly as before.
        assert_eq!(
            normalize_remote("https://user:token@host/o/r").as_deref(),
            Some("host/o/r"),
            "a real credential prefix (before the first `/`) is still stripped"
        );
    }

    #[test]
    fn unusable_remotes_normalize_to_none() {
        for url in [
            "",
            "   ",
            "not-a-url",
            "https://github.com/",
            "https://github.com//x",
        ] {
            assert_eq!(normalize_remote(url), None, "remote {url:?}");
        }
    }

    #[test]
    fn the_stream_id_is_deterministic_path_safe_and_host_scoped() {
        let a = stream_id_from_remote("git@github.com:nxsflow/manufakt-io.git").unwrap();
        let b = stream_id_from_remote("https://github.com/nxsflow/manufakt-io").unwrap();
        assert_eq!(a, b, "every clone derives the same id — no --join needed");
        assert!(a.starts_with("stream-"));
        assert_eq!(a.len(), "stream-".len() + 24);
        assert!(is_valid_stream_id(&a), "path-safe by construction");

        let other_forge = stream_id_from_remote("git@gitlab.com:nxsflow/manufakt-io.git").unwrap();
        assert_ne!(a, other_forge, "the host is part of the identity");
    }

    #[test]
    fn the_id_gate_accepts_derived_and_minted_ids_and_rejects_url_hostile_ones() {
        assert!(is_valid_stream_id("stream-3f9a2c1d8b7e4a05c6d1e2f3"));
        assert!(is_valid_stream_id("nxsflow__manufakt-io"));
        assert!(is_valid_stream_id("a.b-c_d"));
        for bad in [
            "",
            "nxsflow/manufakt-io",
            "a?b",
            "a#b",
            "a b",
            "UPPER",
            "a%2Fb",
            // Path segments a URL normalises away — `{base}/streams/../ops` is `{base}/ops`.
            ".",
            "..",
        ] {
            assert!(!is_valid_stream_id(bad), "must reject {bad:?}");
        }
    }
}
