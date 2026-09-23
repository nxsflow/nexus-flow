//! aye.24 — the on-ramp for an existing Claude-host memory store. `nxm prime` forbids a
//! `MEMORY.md` (durable knowledge lives only in `nxm remember`); this module migrates whatever a
//! project already accumulated in Claude's own memory directory INTO nexus-memory, so the
//! prohibition is an on-ramp instead of a dead end.
//!
//! The source is Claude Code's per-project memory directory — `~/.claude/projects/<slug>/memory/`
//! with a `MEMORY.md` index plus one markdown file per fact, each carrying YAML-ish frontmatter
//! (`name`, `description`, `metadata.type`). The frontmatter `name:` is the fact's stable host id,
//! so it becomes the nexus-memory key verbatim: re-running the import upserts in place (idempotent)
//! rather than duplicating. Reading only — the source is never modified (non-destructive).

use std::path::{Path, PathBuf};

/// The index file that anchors a Claude-host memory directory. Its presence is the detection
/// signal; it is itself skipped on import (it lists the facts, it is not one).
pub const MEMORY_INDEX: &str = "MEMORY.md";

/// One memory parsed from a Claude-host source file, ready to hand to `nxm remember`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedFact {
    /// The nexus-memory key: the frontmatter `name:` (the host's own stable id), else a fallback.
    pub key: String,
    /// The fact text — the file body after the frontmatter, trimmed.
    pub body: String,
}

/// Parse one Claude-host memory file. The key is the frontmatter `name:` slug (the host's stable
/// id), else `fallback_key` (the caller passes the file stem). The body is everything after the
/// closing frontmatter fence, trimmed. Returns `None` when the file has no leading frontmatter
/// (not a structured fact — e.g. the `MEMORY.md` index or a freeform note) or an empty body.
pub fn parse_fact(content: &str, fallback_key: &str) -> Option<ImportedFact> {
    let lines: Vec<&str> = content.lines().collect();
    // The frontmatter must open the file (after any leading blank lines).
    let open = lines.iter().position(|l| !l.trim().is_empty())?;
    if lines[open].trim() != "---" {
        return None; // no frontmatter fence → not a structured fact
    }
    // The closing fence ends the frontmatter; the rest is the body.
    let close = open + 1 + lines[open + 1..].iter().position(|l| l.trim() == "---")?;
    let frontmatter = &lines[open + 1..close];
    let body = lines[close + 1..].join("\n").trim().to_string();
    if body.is_empty() {
        return None; // nothing to import
    }
    // The stable key is the top-level `name:` (column 0, so a nested `  type:` never matches),
    // else the caller's fallback (the file stem).
    let key = frontmatter
        .iter()
        .find_map(|l| l.strip_prefix("name:").map(|v| v.trim().to_string()))
        .filter(|k| !k.is_empty())
        .unwrap_or_else(|| fallback_key.to_string());
    Some(ImportedFact { key, body })
}

/// Map beads memories (`bd export`, `_type == "memory"`) to [`ImportedFact`]s for the SAME upsert
/// core the Claude-host import uses (nexus-flow-6ef.4) — beads is just a second source, not a second
/// write path. The beads `key` is the stable nxm key (preserved verbatim, so a re-import upserts in
/// place); the `value` is the body. Memories with an empty body are skipped (mirroring
/// [`parse_fact`]); the export carries no timestamp, so the caller stamps the migration clock.
pub fn beads_facts(memories: &[nxs_init::beads::BeadsMemory]) -> Vec<ImportedFact> {
    memories
        .iter()
        .filter_map(|m| {
            let body = m.value.trim();
            (!body.is_empty()).then(|| ImportedFact {
                key: m.key.clone(),
                body: body.to_string(),
            })
        })
        .collect()
}

/// Scan a Claude-host memory directory: every `*.md` fact file except the `MEMORY.md` index,
/// parsed and returned sorted by key (deterministic, agent-ergonomic). Read-only — the source is
/// never touched. A missing/unreadable directory yields an empty list.
pub fn scan(dir: &Path) -> Vec<ImportedFact> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter(|p| p.file_name().is_some_and(|n| n != MEMORY_INDEX))
        .collect();
    files.sort();
    let mut out: Vec<ImportedFact> = files
        .iter()
        .filter_map(|f| {
            let content = std::fs::read_to_string(f).ok()?;
            let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            parse_fact(&content, stem)
        })
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

/// Detect a Claude-host memory source for a project rooted at `cwd`: the `env_override` path if set
/// (the `NXM_CLAUDE_MEMORY_DIR` escape hatch / test seam), else `<home>/.claude/projects/<slug>/
/// memory`, where `<slug>` is `cwd` with every non-alphanumeric byte mapped to `-` (Claude Code's
/// own project-dir scheme). Returns `Some(dir)` only when the directory carries a [`MEMORY_INDEX`]
/// anchor, so a project with no Claude memory produces no noise.
pub fn detect(cwd: &Path, home: Option<&Path>, env_override: Option<&str>) -> Option<PathBuf> {
    let dir = match env_override.filter(|s| !s.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => home?
            .join(".claude")
            .join("projects")
            .join(host_slug(cwd))
            .join("memory"),
    };
    dir.join(MEMORY_INDEX).is_file().then_some(dir)
}

/// Claude Code's per-project directory slug: the absolute path with every non-alphanumeric byte
/// replaced by `-` (e.g. `/Users/x/dev/proj` → `-Users-x-dev-proj`).
fn host_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const FRONTMATTER_FACT: &str = "\
---
name: auth-jwt
description: how auth works
metadata:
  type: project
---

auth uses JWT, refresh tokens live in Redis.
";

    #[test]
    fn parse_fact_takes_the_frontmatter_name_as_key_and_the_body_after() {
        let f = parse_fact(FRONTMATTER_FACT, "fallback").expect("a structured fact parses");
        assert_eq!(f.key, "auth-jwt", "the frontmatter name is the stable key");
        assert_eq!(
            f.body, "auth uses JWT, refresh tokens live in Redis.",
            "the body is the content after the frontmatter, trimmed"
        );
    }

    #[test]
    fn parse_fact_falls_back_to_the_stem_when_frontmatter_has_no_name() {
        let content = "---\ndescription: x\n---\n\nsome durable fact\n";
        let f = parse_fact(content, "the-file-stem").expect("parses with a fallback key");
        assert_eq!(f.key, "the-file-stem");
        assert_eq!(f.body, "some durable fact");
    }

    #[test]
    fn parse_fact_skips_a_file_without_frontmatter() {
        // The MEMORY.md index and freeform notes have no leading `---` fence — not facts.
        assert_eq!(parse_fact("- [Title](x.md) — hook\n", "x"), None);
        assert_eq!(parse_fact("just some prose\n", "x"), None);
    }

    #[test]
    fn parse_fact_skips_an_empty_body() {
        assert_eq!(parse_fact("---\nname: k\n---\n\n   \n", "k"), None);
    }

    #[test]
    fn scan_returns_fact_files_sorted_by_key_excluding_the_index() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("MEMORY.md"), "- [A](a.md)\n").unwrap();
        fs::write(
            dir.path().join("zebra.md"),
            "---\nname: zebra\n---\n\nstripes\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("alpha.md"),
            "---\nname: alpha\n---\n\nfirst\n",
        )
        .unwrap();
        // A no-frontmatter note is ignored.
        fs::write(dir.path().join("note.md"), "freeform, not a fact\n").unwrap();

        let facts = scan(dir.path());
        let keys: Vec<&str> = facts.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            ["alpha", "zebra"],
            "key-sorted, index + note excluded"
        );
        assert_eq!(facts[0].body, "first");
    }

    #[test]
    fn scan_of_a_missing_directory_is_empty() {
        assert!(scan(Path::new("/no/such/dir/whatsoever")).is_empty());
    }

    #[test]
    fn detect_prefers_the_env_override_when_it_anchors_a_memory_index() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(MEMORY_INDEX), "index\n").unwrap();
        let found = detect(
            Path::new("/whatever/cwd"),
            None,
            Some(dir.path().to_str().unwrap()),
        );
        assert_eq!(found.as_deref(), Some(dir.path()));
    }

    #[test]
    fn detect_returns_none_without_a_memory_index_anchor() {
        let dir = TempDir::new().unwrap(); // exists, but no MEMORY.md
        assert_eq!(
            detect(
                Path::new("/whatever"),
                None,
                Some(dir.path().to_str().unwrap())
            ),
            None,
            "an empty/foreign dir is not a Claude memory source"
        );
    }

    #[test]
    fn detect_derives_the_host_slug_path_under_home() {
        let home = TempDir::new().unwrap();
        let cwd = Path::new("/Users/x/dev/proj");
        let mem = home
            .path()
            .join(".claude")
            .join("projects")
            .join("-Users-x-dev-proj")
            .join("memory");
        fs::create_dir_all(&mem).unwrap();
        fs::write(mem.join(MEMORY_INDEX), "index\n").unwrap();

        let found = detect(cwd, Some(home.path()), None);
        assert_eq!(found.as_deref(), Some(mem.as_path()));
    }

    #[test]
    fn beads_facts_preserves_keys_and_skips_empty_bodies() {
        let memories = vec![
            nxs_init::beads::BeadsMemory {
                key: "changelog-scope".into(),
                value: "  fragments feed release-notes  ".into(),
            },
            nxs_init::beads::BeadsMemory {
                key: "empty".into(),
                value: "   ".into(),
            },
        ];
        let facts = beads_facts(&memories);
        assert_eq!(facts.len(), 1, "the empty-bodied memory is skipped");
        assert_eq!(facts[0].key, "changelog-scope", "key preserved verbatim");
        assert_eq!(
            facts[0].body, "fragments feed release-notes",
            "body trimmed, like the Claude-host path"
        );
    }

    #[test]
    fn host_slug_maps_non_alphanumerics_to_dashes() {
        assert_eq!(
            host_slug(Path::new("/Users/ckoch/Development/nxsflow/nexus-flow")),
            "-Users-ckoch-Development-nxsflow-nexus-flow"
        );
    }
}
