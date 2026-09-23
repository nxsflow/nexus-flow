//! Where a workspace syncs to (spec §4.2). Two levels — a global default plus a
//! per-workspace override — resolved by ONE function so the precedence cannot drift.

use crate::error::{ErrorKind, NxfError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default)]
    pub sync: SyncSection,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SyncSection {
    #[serde(default)]
    pub default_endpoint: Option<String>,
}

/// The precedence, in one place:
///   `run --remote` (one-shot, never persisted)
///     > `.nxs/sync.toml` endpoint (per workspace)
///       > `~/.nexusflow/config.toml` [sync] default_endpoint
///         > a loud error naming both ways to set one.
pub fn resolve(
    one_shot: Option<&str>,
    workspace: Option<&str>,
    global: Option<&str>,
) -> Result<String> {
    one_shot
        .or(workspace)
        .or(global)
        .map(str::to_string)
        .ok_or_else(|| {
            NxfError::new(
                ErrorKind::Validation,
                "no sync endpoint for this workspace; set a global default with \
                 `nxs sync endpoint <url>`, or bind one for this workspace with \
                 `nxs sync bind --endpoint <url>` (or pass `--remote <url>` for one run)",
            )
        })
}

/// Read the global default. An absent or empty file is "unset", not an error; a present
/// but malformed file is refused loudly rather than silently read as unset (the same
/// no-silent-guessing rule the workspace registry follows).
pub fn load_global_from(path: &Path) -> Result<Option<String>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(NxfError::io(format!("reading {}: {e}", path.display()))),
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    // A TOML error quotes the offending line, and that line can be the endpoint with its key —
    // on its way into the service log (review of PR #487, Integrity #2).
    let cfg: GlobalConfig = toml::from_str(&raw).map_err(|e| {
        NxfError::validation(nxs_sync::redact::redact_userinfo(&format!(
            "{} is not valid TOML — refusing to guess its contents: {e}",
            path.display()
        )))
    })?;
    Ok(cfg.sync.default_endpoint)
}

/// Persist the global default, preserving anything else already in the file. Deliberately does
/// NOT round-trip through the typed [`GlobalConfig`]: serde silently drops fields a struct doesn't
/// declare, so writing back a re-serialized struct would erase any sibling key in `[sync]` or any
/// unrelated top-level section — quietly, on every save. This is shared machine-wide config, not a
/// file this module owns exclusively, so it edits a generic [`toml::Value`] document instead,
/// touching only the one key it is responsible for.
///
/// Only a genuinely ABSENT file reads as "start from an empty document" — any other read failure
/// (permission denied, a symlink loop, the path being a directory, ...) is propagated rather than
/// swallowed, exactly like [`load_global_from`]. Treating every read error as "nothing here yet"
/// would silently discard whatever real content is actually there, defeating the very
/// preserve-other-content guarantee this function exists to provide.
///
/// The write itself goes through [`crate::workspaces::write_atomic`] (temp file, `O_EXCL`, rename)
/// rather than a plain `fs::write`: `~/.nexusflow/` is a shared-home, multi-writer directory — the
/// same threat model `workspaces.toml` sits under — so a symlink squatting on the predictable temp
/// path must be refused rather than followed. Reusing that helper keeps the hardening in ONE place
/// instead of a second, divergent copy.
pub fn save_global_to(path: &Path, url: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| NxfError::io(format!("creating {}: {e}", parent.display())))?;
    }
    let mut doc: toml::Value = match std::fs::read_to_string(path) {
        Ok(raw) if raw.trim().is_empty() => toml::Value::Table(toml::value::Table::new()),
        Ok(raw) => toml::from_str(&raw).map_err(|e| {
            NxfError::validation(nxs_sync::redact::redact_userinfo(&format!(
                "{} is not valid TOML: {e}",
                path.display()
            )))
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            toml::Value::Table(toml::value::Table::new())
        }
        Err(e) => return Err(NxfError::io(format!("reading {}: {e}", path.display()))),
    };
    let table = doc.as_table_mut().ok_or_else(|| {
        NxfError::validation(format!(
            "{} is not a TOML table at the top level",
            path.display()
        ))
    })?;
    let sync_table = table
        .entry("sync")
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        .ok_or_else(|| {
            NxfError::validation(format!("{}: [sync] is not a table", path.display()))
        })?;
    sync_table.insert(
        "default_endpoint".to_string(),
        toml::Value::String(url.to_string()),
    );
    let raw = toml::to_string(&doc)
        .map_err(|e| NxfError::io(format!("serializing the global config: {e}")))?;
    // Owner-only: the endpoint can carry a relay's key (review of PR #487, Integrity #2).
    crate::workspaces::write_atomic_private(path, raw.as_bytes())
}

/// `~/.nexusflow/config.toml` — from [`ServiceHome`](nxs_service::ServiceHome), NOT from a copy of
/// the directory name kept here (nxf 6j6v.gd9p).
///
/// This module held the fifth independent spelling of `~/.nexusflow` while `home.rs`'s own doc
/// claimed the directory was "named once". It was the copy that made the claim false, and it is the
/// one that would have kept a named instance reading the SHARED global config while its registry,
/// lock and heartbeat had already moved — a half-isolated service, which is worse than none.
pub fn config_path() -> Result<PathBuf> {
    Ok(nxs_service::ServiceHome::resolve()?.config())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    #[test]
    fn the_one_shot_override_beats_everything() {
        let got = resolve(
            Some("http://one-shot"),
            Some("http://ws"),
            Some("http://global"),
        )
        .unwrap();
        assert_eq!(got, "http://one-shot");
    }

    #[test]
    fn the_workspace_override_beats_the_global_default() {
        assert_eq!(
            resolve(None, Some("http://ws"), Some("http://global")).unwrap(),
            "http://ws"
        );
    }

    #[test]
    fn the_global_default_applies_when_the_workspace_has_no_override() {
        assert_eq!(
            resolve(None, None, Some("http://global")).unwrap(),
            "http://global"
        );
    }

    #[test]
    fn no_endpoint_anywhere_is_a_loud_error_naming_both_ways_to_set_one() {
        let err = resolve(None, None, None).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(
            err.msg.contains("nxs sync endpoint"),
            "names the global setter: {}",
            err.msg
        );
        assert!(
            err.msg.contains("--endpoint"),
            "names the per-workspace flag: {}",
            err.msg
        );
    }

    #[test]
    fn the_global_config_round_trips_and_a_missing_file_is_not_an_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        assert_eq!(
            load_global_from(&path).unwrap(),
            None,
            "absent file = unset, not an error"
        );
        save_global_to(&path, "https://relay.example").unwrap();
        assert_eq!(
            load_global_from(&path).unwrap().as_deref(),
            Some("https://relay.example")
        );
        save_global_to(&path, "https://other.example").unwrap();
        assert_eq!(
            load_global_from(&path).unwrap().as_deref(),
            Some("https://other.example")
        );
    }

    #[test]
    fn a_malformed_global_config_is_refused_loudly_rather_than_read_as_unset() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "this is not toml {{{").unwrap();
        let err = load_global_from(&path).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
    }

    #[test]
    fn save_global_to_preserves_content_it_does_not_own() {
        // The config file is shared machine-wide state (§ task-5 constraint), not a single-purpose
        // file — an unrelated section, and an unrelated key alongside `default_endpoint` inside
        // `[sync]` itself, must both survive a save untouched.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            "[sync]\ndefault_endpoint = \"http://old\"\nsome_other_key = \"keep-me\"\n\n\
             [unrelated]\nfoo = 1\n",
        )
        .unwrap();
        save_global_to(&path, "http://new").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            raw.contains("some_other_key"),
            "sibling sync key kept: {raw}"
        );
        assert!(raw.contains("keep-me"), "sibling sync value kept: {raw}");
        assert!(raw.contains("[unrelated]"), "unrelated section kept: {raw}");
        assert!(
            raw.contains("foo"),
            "unrelated section's content kept: {raw}"
        );
        assert_eq!(
            load_global_from(&path).unwrap().as_deref(),
            Some("http://new"),
            "the field this function owns still updated"
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_global_to_refuses_rather_than_clobbers_on_a_non_not_found_read_error() {
        // A read failure that is NOT "file absent" — here, no read permission on an existing file —
        // must be a loud error, never silently treated as "nothing here yet". Swallowing it would
        // let the save proceed to write a fresh document over content it never actually read,
        // silently discarding it; the file's real bytes must survive a refused save.
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        let original = "[sync]\ndefault_endpoint = \"http://original\"\nkeep = \"me\"\n";
        std::fs::write(&path, original).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o200)).unwrap(); // write-only

        let result = save_global_to(&path, "http://new");
        // Restore permissions before asserting so the tempdir can clean itself up either way.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let err = result.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Io);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "the unreadable file's real content must survive a refused save"
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_global_to_refuses_a_pre_placed_temp_symlink_and_leaves_the_target_untouched() {
        // Mirrors workspaces.rs's
        // `upsert_refuses_a_pre_placed_temp_symlink_and_leaves_the_target_untouched`: `config.toml`
        // sits in the same shared-home, multi-writer `~/.nexusflow/` dir under the same TOCTOU
        // threat model, and now reuses the same `write_atomic` hardening, so it must resist the
        // same attack — a symlink squatting on the predictable temp path must not be followed.
        use std::os::unix::fs::symlink;
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");

        let outside = tmp.path().join("precious.txt");
        std::fs::write(&outside, b"PRECIOUS").unwrap();

        // The exact predictable temp path `write_atomic` computes: `.{file_name}.nxs.tmp.{pid}`.
        let temp_path = tmp
            .path()
            .join(format!(".config.toml.nxs.tmp.{}", std::process::id()));
        symlink(&outside, &temp_path).unwrap();

        let err = save_global_to(&path, "http://new").unwrap_err();
        assert_eq!(
            err.kind,
            ErrorKind::Io,
            "a squatted temp path is a loud io error"
        );
        assert_eq!(
            std::fs::read(&outside).unwrap(),
            b"PRECIOUS",
            "the symlink target is never written through"
        );
        assert!(
            !path.exists(),
            "no config file was produced through the symlink"
        );
    }
}
