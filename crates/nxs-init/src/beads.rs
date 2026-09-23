//! The shared beads-migration source layer (nexus-flow-6ef) — the one place that knows beads' own
//! artifact shapes, so the orchestration (`nxs init`), the flow ticket import, and the memory
//! import all read the SAME parsed records and detect the SAME signals.
//!
//! It is deliberately **vocabulary-free and dependency-light**: pure serde structs over the
//! `bd export` JSONL plus pure detection/rebuild predicates, no flow/memory types and no core
//! store. The flow side ([`crate::beads`] consumers in `nexus-flow-cli`) maps [`BeadsIssue`] onto
//! the task model; the memory side maps [`BeadsMemory`] onto the fact model; `nxs` orchestrates.
//!
//! **Source = backup.** `bd export --all` emits one JSONL with a `_type`-tagged record per line —
//! `issue` and `memory` are the two we carry; every other `_type` (infra/templates/gates) is
//! skipped (forward-compatible). The same artifact is the timestamped backup written on consent.

use serde::Deserialize;

/// The beads managed-block BEGIN marker prefix (line-anchored) in `AGENTS.md`/`CLAUDE.md`. beads
/// writes `<!-- BEGIN BEADS INTEGRATION v:1 … -->`; the version/profile/hash tail varies, so only
/// the stable prefix is matched.
pub const BLOCK_BEGIN: &str = "<!-- BEGIN BEADS INTEGRATION";
/// The beads managed-block END marker (full line). Distinct from the assembler's `markers_of`
/// derivation (which would key on the first token `BEADS`, not the full `BEADS INTEGRATION`).
pub const BLOCK_END: &str = "<!-- END BEADS INTEGRATION -->";
/// The beads SessionStart/PreCompact hook command in `.claude/settings.json`.
pub const HOOK_COMMAND: &str = "bd prime";

/// One ticket from `bd export` (`_type == "issue"`). Optional/absent fields default so a sparse
/// export line (e.g. an open issue with no `closed_at`/`close_reason`) still parses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct BeadsIssue {
    /// The beads short id (e.g. `nexus-flow-aye.40`) — the key the import's id-translation table
    /// maps to a freshly minted flow id.
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// beads lifecycle: `open` | `closed` (and `in_progress` if a board uses it).
    #[serde(default)]
    pub status: String,
    /// beads numeric priority `0..=4` (0 = highest). `None` when absent.
    #[serde(default)]
    pub priority: Option<i64>,
    /// beads type: `bug` | `feature` | `epic` | `task` | `chore` | `decision`.
    #[serde(default)]
    pub issue_type: String,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub design: Option<String>,
    /// beads' acceptance criteria — maps to flow's `completion_criterion` (definition of done).
    #[serde(default)]
    pub acceptance_criteria: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub closed_at: Option<String>,
    /// The closing comment recorded by beads — `None`/empty when an issue was closed without one
    /// (the importer supplies a default, since flow requires a reason at close).
    #[serde(default)]
    pub close_reason: Option<String>,
    /// beads' defer-until instant (`defer_until` in the export) — the ticket is hidden from
    /// `bd ready` until then. Maps 1:1 to flow's `defer_until` (a5w: it used to be dropped here,
    /// so a deferred ticket migrated as plain `open`).
    #[serde(default)]
    pub defer_until: Option<String>,
    /// beads' due date (`due_at` in the current export; older/newer exports may use `due`, accepted
    /// via the alias). Maps to flow's `due` (a5w's sibling gap — the same silent-drop class).
    #[serde(default, rename = "due_at", alias = "due")]
    pub due: Option<String>,
    /// beads' user labels (free-text tags). Migrate into flow's user-label OR-set (h89s.4); an
    /// export without labels yields an empty vec, so the migration is unchanged for unlabelled boards.
    #[serde(default)]
    pub labels: Vec<String>,
    /// The directed relationships this issue carries (parent-child, blocks, related, …).
    #[serde(default)]
    pub dependencies: Vec<BeadsDep>,
}

/// One relationship edge inside a [`BeadsIssue`]. `dep_type` categorizes it; `depends_on_id` is the
/// OTHER endpoint, with beads' convention that `issue_id` depends-on / is-blocked-by / is-a-child-of
/// `depends_on_id` (verified against the live export: a `parent-child` edge's `depends_on_id` IS the
/// parent; a `blocks` edge means `issue_id` depends on `depends_on_id`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct BeadsDep {
    #[serde(default)]
    pub issue_id: String,
    pub depends_on_id: String,
    #[serde(rename = "type", default)]
    pub dep_type: String,
}

/// One memory from `bd export` (`_type == "memory"`). beads' `bd remember` carries only a stable
/// `key` and a `value`; the export records NO timestamp or active flag, so the memory import stamps
/// the migration clock and treats every exported memory as active (forgotten memories are absent).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct BeadsMemory {
    pub key: String,
    #[serde(default)]
    pub value: String,
}

/// The two record kinds the migration carries, parsed from one `bd export --all` JSONL artifact.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BeadsExport {
    pub issues: Vec<BeadsIssue>,
    pub memories: Vec<BeadsMemory>,
}

/// Parse a `bd export --all` JSONL artifact into its issues + memories. Blank lines are skipped;
/// each non-blank line is dispatched on its `_type` (`issue`/`memory` carried, every other type
/// skipped for forward-compatibility). A line that IS an issue/memory but fails to deserialize is a
/// loud error naming the line — a malformed ticket is never silently dropped (the artifact is also
/// the user's backup, so fidelity beats best-effort).
pub fn parse_export(jsonl: &str) -> nxs_foundation::error::Result<BeadsExport> {
    use nxs_foundation::error::NxfError;
    let mut export = BeadsExport::default();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| NxfError::validation(format!("malformed bd export line: {e}: {line}")))?;
        match value.get("_type").and_then(serde_json::Value::as_str) {
            Some("issue") => export
                .issues
                .push(serde_json::from_value(value).map_err(|e| {
                    NxfError::validation(format!("malformed beads issue record: {e}: {line}"))
                })?),
            Some("memory") => export
                .memories
                .push(serde_json::from_value(value).map_err(|e| {
                    NxfError::validation(format!("malformed beads memory record: {e}: {line}"))
                })?),
            // Every other record kind (infra/templates/gates) is intentionally skipped — the
            // migration carries only tickets + memories.
            _ => {}
        }
    }
    Ok(export)
}

// ── detection (6ef.1) ───────────────────────────────────────────────────────

/// Which beads signals a project carries. The migration is offered when [`BeadsSignals::present`]
/// — i.e. beads is still WIRED (a managed block or the `bd prime` hook) or an `issues.jsonl` export
/// is sitting there. Deliberately NOT the bare `.beads/` directory: a successful migration leaves
/// `.beads/` in place as the untouched backup, and re-detecting on that alone would re-offer the
/// migration forever. Block + hook are removed by the rebuild and `issues.jsonl` is never recreated,
/// so a re-run after migrating sees no signal — idempotent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BeadsSignals {
    /// A `.beads/issues.jsonl` export file is present.
    pub issues_jsonl: bool,
    /// A beads managed block is present in `AGENTS.md` or `CLAUDE.md`.
    pub managed_block: bool,
    /// A `bd prime` hook is wired in `.claude/settings.json`.
    pub session_hook: bool,
}

impl BeadsSignals {
    /// Whether any signal fired — beads is present and the migration should be offered.
    pub fn present(self) -> bool {
        self.issues_jsonl || self.managed_block || self.session_hook
    }
}

/// Whether `content` carries a beads managed block — a [`BLOCK_BEGIN`] marker at column 0 (so a
/// marker quoted inside prose/code is never matched, mirroring the assembler's line-anchoring).
pub fn content_has_block(content: &str) -> bool {
    line_anchored(content, BLOCK_BEGIN).is_some()
}

/// Whether parsed `.claude/settings.json` wires a `bd prime` hook under ANY event (SessionStart,
/// PreCompact, …) — beads installs it on both, and any one is a present-signal.
pub fn settings_has_hook(settings: &serde_json::Value) -> bool {
    let Some(events) = settings.get("hooks").and_then(serde_json::Value::as_object) else {
        return false;
    };
    events.values().any(|groups| {
        groups.as_array().is_some_and(|gs| {
            gs.iter().any(|g| {
                g.get("hooks")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|hs| {
                        hs.iter().any(|h| {
                            h.get("command").and_then(serde_json::Value::as_str)
                                == Some(HOOK_COMMAND)
                        })
                    })
            })
        })
    })
}

/// Detect beads signals for the project rooted at `root` (FS-facing): an `issues.jsonl` export, a
/// managed block in `AGENTS.md`/`CLAUDE.md`, or a `bd prime` hook in `.claude/settings.json`. Each
/// missing/unreadable input simply contributes `false` — detection is quiet, never an error.
pub fn detect(root: &std::path::Path) -> BeadsSignals {
    let read = |rel: &str| std::fs::read_to_string(root.join(rel)).ok();
    let managed_block = [read("AGENTS.md"), read("CLAUDE.md")]
        .into_iter()
        .flatten()
        .any(|c| content_has_block(&c));
    let session_hook = read(".claude/settings.json")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .is_some_and(|v| settings_has_hook(&v));
    BeadsSignals {
        issues_jsonl: root.join(".beads/issues.jsonl").is_file(),
        managed_block,
        session_hook,
    }
}

// ── config rollback transforms (6ef.5) ───────────────────────────────────────

/// Remove the beads managed block (`<!-- BEGIN BEADS INTEGRATION … <!-- END BEADS INTEGRATION -->`)
/// from `content`, returning `(new_content, removed)`. ONLY the marked, line-anchored block is
/// touched — hand-written prose outside it is never edited (the migration's leitplanke). When a
/// block is removed, runs of 3+ newlines collapse to a single blank so no widening gap is left; a
/// blockless file is returned byte-for-byte. A `BEGIN` with no matching `END` is a corrupted managed
/// region → a loud error (fail-loud, never clobber), mirroring the assembler.
pub fn strip_block(content: &str) -> nxs_foundation::error::Result<(String, bool)> {
    use nxs_foundation::error::NxfError;
    let mut out = content.to_string();
    let mut removed = false;
    while let Some(begin) = line_anchored(&out, BLOCK_BEGIN) {
        let end_rel = line_anchored(&out[begin..], BLOCK_END).ok_or_else(|| {
            NxfError::validation(format!(
                "the beads managed block has no matching `{BLOCK_END}` — repair or remove it, \
                 then re-run the migration"
            ))
        })?;
        let end = begin + end_rel + BLOCK_END.len();
        out.replace_range(begin..end, "");
        removed = true;
    }
    Ok(if removed {
        (collapse_blank_runs(&out), true)
    } else {
        (out, false)
    })
}

/// Collapse runs of 3+ newlines to a single blank line (so removing a block leaves no widening
/// gap); paragraph breaks (1–2 newlines) are preserved. Mirrors the assembler's same-named helper.
fn collapse_blank_runs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newlines = 0usize;
    for ch in s.chars() {
        if ch == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push(ch);
            }
        } else {
            newlines = 0;
            out.push(ch);
        }
    }
    out
}

/// Remove every `bd prime` hook from parsed `.claude/settings.json`, across ALL events (beads
/// installs it on both `SessionStart` and `PreCompact`), returning `(new_settings, removed)`. Groups
/// emptied of hooks are dropped, and an event left with no groups is removed — so the rollback
/// leaves no hollow structure. Unrelated hooks/events are untouched (the nxs hook is added
/// separately by the assembler). A settings file with no `bd prime` hook is unchanged.
pub fn strip_hooks(mut settings: serde_json::Value) -> (serde_json::Value, bool) {
    use serde_json::Value;
    let mut removed = false;
    let Some(events) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return (settings, false);
    };
    for groups in events.values_mut() {
        let Some(arr) = groups.as_array_mut() else {
            continue;
        };
        for group in arr.iter_mut() {
            if let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                let before = hooks.len();
                hooks.retain(|h| h.get("command").and_then(Value::as_str) != Some(HOOK_COMMAND));
                removed |= hooks.len() != before;
            }
        }
        // Drop any group left with no hooks, so the rollback leaves no hollow group.
        arr.retain(|g| {
            g.get("hooks")
                .and_then(Value::as_array)
                .map(|a| !a.is_empty())
                .unwrap_or(true)
        });
    }
    // Drop any event whose groups all went away.
    events.retain(|_, v| v.as_array().map(|a| !a.is_empty()).unwrap_or(true));
    (settings, removed)
}

/// Byte offset of `needle` where it BEGINS A LINE (column 0) — the shared line-anchoring the
/// managed-block predicate + rebuild use, so a marker buried mid-line is never matched.
fn line_anchored(content: &str, needle: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = content[from..].find(needle) {
        let at = from + rel;
        if at == 0 || content.as_bytes()[at - 1] == b'\n' {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUE_LINE: &str = r#"{"_type":"issue","id":"nexus-flow-aye.40","title":"T","description":"D","status":"closed","priority":0,"issue_type":"bug","closed_at":"2026-06-22T22:15:57Z","close_reason":"fixed","dependencies":[{"issue_id":"nexus-flow-aye.40","depends_on_id":"nexus-flow-aye","type":"parent-child"}]}"#;
    const MEMORY_LINE: &str =
        r#"{"_type":"memory","key":"changelog-scope","value":"fragments feed release-notes"}"#;

    #[test]
    fn parses_an_issue_record_with_its_fields_and_edges() {
        let export = parse_export(ISSUE_LINE).unwrap();
        assert_eq!(export.issues.len(), 1, "one issue parsed");
        assert!(export.memories.is_empty());
        let i = &export.issues[0];
        assert_eq!(i.id, "nexus-flow-aye.40");
        assert_eq!(i.title, "T");
        assert_eq!(i.status, "closed");
        assert_eq!(i.priority, Some(0));
        assert_eq!(i.issue_type, "bug");
        assert_eq!(i.close_reason.as_deref(), Some("fixed"));
        assert_eq!(i.closed_at.as_deref(), Some("2026-06-22T22:15:57Z"));
        assert_eq!(i.dependencies.len(), 1);
        assert_eq!(i.dependencies[0].depends_on_id, "nexus-flow-aye");
        assert_eq!(i.dependencies[0].dep_type, "parent-child");
    }

    #[test]
    fn parses_defer_until_due_and_labels() {
        // a5w + h89s.4: the deferred/due dates and the label set must deserialize off `bd export`.
        // `defer_until` and `labels` match beads' export keys directly; the due date's export key is
        // `due_at` (accepted via rename, with a `due` alias for older/newer exports).
        let line = r#"{"_type":"issue","id":"x.1","title":"T","description":"D","status":"open","priority":1,"issue_type":"task","defer_until":"2026-08-01T00:00:00Z","due_at":"2026-09-01T00:00:00Z","labels":["urgent","backend"]}"#;
        let i = &parse_export(line).unwrap().issues[0];
        assert_eq!(i.defer_until.as_deref(), Some("2026-08-01T00:00:00Z"));
        assert_eq!(
            i.due.as_deref(),
            Some("2026-09-01T00:00:00Z"),
            "due_at → due"
        );
        assert_eq!(i.labels, vec!["urgent".to_string(), "backend".to_string()]);
    }

    #[test]
    fn accepts_the_legacy_due_key_alias() {
        // Robustness to export-schema drift: a `due` key (rather than `due_at`) still lands.
        let line = r#"{"_type":"issue","id":"x.1","title":"T","description":"","status":"open","priority":2,"issue_type":"task","due":"2026-12-31T00:00:00Z"}"#;
        let i = &parse_export(line).unwrap().issues[0];
        assert_eq!(i.due.as_deref(), Some("2026-12-31T00:00:00Z"));
    }

    #[test]
    fn parses_a_memory_record() {
        let export = parse_export(MEMORY_LINE).unwrap();
        assert_eq!(export.memories.len(), 1, "one memory parsed");
        assert!(export.issues.is_empty());
        assert_eq!(export.memories[0].key, "changelog-scope");
        assert_eq!(export.memories[0].value, "fragments feed release-notes");
    }

    #[test]
    fn carries_both_issues_and_memories_from_one_artifact() {
        let jsonl = format!("{ISSUE_LINE}\n{MEMORY_LINE}\n");
        let export = parse_export(&jsonl).unwrap();
        assert_eq!(export.issues.len(), 1);
        assert_eq!(export.memories.len(), 1);
    }

    #[test]
    fn skips_blank_lines_and_unknown_record_types() {
        // Infra/template/gate records (and any future `_type`) are skipped, not errored — the
        // migration only carries issues + memories. Blank lines (incl. a trailing newline) too.
        let jsonl =
            format!("{ISSUE_LINE}\n\n{{\"_type\":\"agent\",\"id\":\"infra-1\"}}\n{MEMORY_LINE}\n");
        let export = parse_export(&jsonl).unwrap();
        assert_eq!(export.issues.len(), 1, "only the real issue");
        assert_eq!(export.memories.len(), 1, "only the real memory");
    }

    #[test]
    fn an_open_issue_without_close_fields_parses_with_defaults() {
        let line = r#"{"_type":"issue","id":"x.1","title":"open one","description":"","status":"open","priority":2,"issue_type":"task"}"#;
        let export = parse_export(line).unwrap();
        let i = &export.issues[0];
        assert_eq!(i.status, "open");
        assert_eq!(i.close_reason, None);
        assert_eq!(i.closed_at, None);
        assert!(
            i.dependencies.is_empty(),
            "absent dependencies default empty"
        );
    }

    #[test]
    fn a_malformed_issue_line_is_a_loud_error() {
        // A line tagged `issue` whose shape is broken (id is the required field) must fail loudly —
        // the artifact is the user's backup, so a corrupt ticket is never silently dropped.
        let jsonl = r#"{"_type":"issue","title":"no id here"}"#;
        assert!(parse_export(jsonl).is_err(), "missing required id → error");
    }

    // ---- detection ----------------------------------------------------------

    #[test]
    fn content_has_block_matches_only_a_column_zero_begin_marker() {
        let with = "# Agent Instructions\n\n<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal -->\n## Beads\n<!-- END BEADS INTEGRATION -->\n";
        assert!(content_has_block(with), "real managed block detected");
        assert!(!content_has_block("# notes\n\nno markers here\n"));
        // A marker quoted mid-line (e.g. inside prose about the format) is NOT a managed block.
        assert!(
            !content_has_block("we write `<!-- BEGIN BEADS INTEGRATION` to mark it\n"),
            "an indented/mid-line marker is not anchored at column 0"
        );
    }

    #[test]
    fn settings_has_hook_finds_bd_prime_under_any_event() {
        let session = serde_json::json!({
            "hooks": { "SessionStart": [ { "matcher": "", "hooks": [ { "type": "command", "command": "bd prime" } ] } ] }
        });
        assert!(
            settings_has_hook(&session),
            "SessionStart bd prime detected"
        );
        // beads also installs it on PreCompact — that alone is still a present-signal.
        let precompact = serde_json::json!({
            "hooks": { "PreCompact": [ { "matcher": "", "hooks": [ { "type": "command", "command": "bd prime" } ] } ] }
        });
        assert!(
            settings_has_hook(&precompact),
            "PreCompact bd prime detected"
        );
        // An unrelated hook is not a beads signal.
        let other = serde_json::json!({
            "hooks": { "SessionStart": [ { "hooks": [ { "command": "nxs prime" } ] } ] }
        });
        assert!(!settings_has_hook(&other), "nxs prime is not a beads hook");
        assert!(!settings_has_hook(&serde_json::json!({})), "empty → none");
    }

    #[test]
    fn signals_present_iff_any_signal_fired() {
        assert!(!BeadsSignals::default().present(), "no signal → absent");
        assert!(BeadsSignals {
            managed_block: true,
            ..Default::default()
        }
        .present());
        assert!(BeadsSignals {
            session_hook: true,
            ..Default::default()
        }
        .present());
    }

    #[test]
    fn detect_reads_the_three_signals_from_disk() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        // issues.jsonl under .beads/
        std::fs::create_dir_all(root.join(".beads")).unwrap();
        std::fs::write(root.join(".beads/issues.jsonl"), "{}\n").unwrap();
        // a managed block in CLAUDE.md
        std::fs::write(
            root.join("CLAUDE.md"),
            "# x\n<!-- BEGIN BEADS INTEGRATION v:1 -->\nb\n<!-- END BEADS INTEGRATION -->\n",
        )
        .unwrap();
        // a bd prime SessionStart hook
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(
            root.join(".claude/settings.json"),
            r#"{"hooks":{"SessionStart":[{"hooks":[{"command":"bd prime"}]}]}}"#,
        )
        .unwrap();

        let s = detect(root);
        assert!(
            s.issues_jsonl && s.managed_block && s.session_hook,
            "all three: {s:?}"
        );
        assert!(s.present());
    }

    #[test]
    fn detect_on_a_clean_project_finds_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(!detect(tmp.path()).present(), "no beads → no signal");
    }

    #[test]
    fn detect_ignores_a_bare_beads_dir_left_as_backup() {
        // The idempotence guard: a successful migration leaves `.beads/` (data/backup) but removes
        // the block + hook and never recreates issues.jsonl. A bare `.beads/` must NOT re-trigger.
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".beads/backup")).unwrap();
        assert!(
            !detect(tmp.path()).present(),
            "a bare .beads/ dir (post-migration backup) is not a present-signal"
        );
    }

    // ---- config rollback transforms (6ef.5) ---------------------------------

    #[test]
    fn strip_block_removes_only_the_managed_block_and_keeps_prose() {
        let content = "# Agent Instructions\n\nHand-written intro about bd.\n\n<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:abc -->\n## Beads\nmanaged stuff\n<!-- END BEADS INTEGRATION -->\n\n## My own section\nkeep me\n";
        let (out, removed) = strip_block(content).unwrap();
        assert!(removed, "a block was removed");
        assert!(
            !out.contains("BEGIN BEADS INTEGRATION"),
            "block gone: {out}"
        );
        assert!(!out.contains("managed stuff"), "block body gone: {out}");
        assert!(
            out.contains("Hand-written intro about bd."),
            "prose kept: {out}"
        );
        assert!(
            out.contains("## My own section") && out.contains("keep me"),
            "kept: {out}"
        );
        assert!(!out.contains("\n\n\n"), "no widening blank gap: {out:?}");
    }

    #[test]
    fn strip_block_leaves_a_blockless_file_byte_for_byte() {
        let content = "# notes\n\n\nintentional gap, no managed block\n";
        let (out, removed) = strip_block(content).unwrap();
        assert!(!removed);
        assert_eq!(
            out, content,
            "no block ⇒ byte-identical (never normalizes user whitespace)"
        );
    }

    #[test]
    fn strip_block_errors_on_an_unterminated_block() {
        let content = "# x\n<!-- BEGIN BEADS INTEGRATION v:1 -->\nbody, no end\n";
        assert!(
            strip_block(content).is_err(),
            "a BEGIN with no END is a loud error"
        );
    }

    #[test]
    fn strip_hooks_removes_bd_prime_from_every_event() {
        let settings = serde_json::json!({
            "hooks": {
                "SessionStart": [ { "matcher": "", "hooks": [ { "type": "command", "command": "bd prime" } ] } ],
                "PreCompact": [ { "matcher": "", "hooks": [ { "type": "command", "command": "bd prime" } ] } ]
            },
            "model": "opus"
        });
        let (out, removed) = strip_hooks(settings);
        assert!(removed, "bd prime hooks removed");
        // Both events were sole-occupied by bd prime → removed entirely; unrelated keys kept.
        assert!(
            out["hooks"].get("SessionStart").is_none(),
            "empty event dropped: {out}"
        );
        assert!(
            out["hooks"].get("PreCompact").is_none(),
            "empty event dropped: {out}"
        );
        assert_eq!(out["model"], "opus", "unrelated keys preserved");
    }

    #[test]
    fn strip_hooks_preserves_unrelated_hooks_in_the_same_event() {
        let settings = serde_json::json!({
            "hooks": { "SessionStart": [
                { "matcher": "", "hooks": [ { "type": "command", "command": "bd prime" } ] },
                { "matcher": "", "hooks": [ { "type": "command", "command": "nxs prime" } ] }
            ] }
        });
        let (out, removed) = strip_hooks(settings);
        assert!(removed);
        let cmds: Vec<&str> = out["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["hooks"].as_array().unwrap())
            .filter_map(|h| h["command"].as_str())
            .collect();
        assert_eq!(
            cmds,
            vec!["nxs prime"],
            "only bd prime removed; nxs prime kept"
        );
    }

    #[test]
    fn strip_hooks_without_bd_prime_changes_nothing() {
        let settings = serde_json::json!({
            "hooks": { "SessionStart": [ { "hooks": [ { "command": "nxs prime" } ] } ] }
        });
        let (_out, removed) = strip_hooks(settings);
        assert!(!removed, "no bd prime → no change");
    }
}
