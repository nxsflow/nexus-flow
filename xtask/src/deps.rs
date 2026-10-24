//! `cargo xtask deps …` (nexus-flow-6j6v.2dn0): the two halves of the dependency vulnerability
//! gate that no off-the-shelf tool carries — see docs/runbooks/dependency-vulnerabilities.md.
//!
//! `cargo deny check` refuses an advisory against a shipped Rust crate, and an advisory that is
//! accepted for now is an `[advisories] ignore` entry in `deny.toml`. cargo-deny takes exactly two
//! keys there, `id` and `reason`, so it has no notion of an exception running OUT: an ignore entry
//! written today silences that advisory for good. `deps exceptions` is the missing half. Every
//! entry must carry its expiry and its board item at the head of its reason —
//!
//! ```toml
//! { id = "RUSTSEC-2024-0449", reason = "expires 2027-01-31: <why it is accepted> (6j6v.xxxx)" }
//! ```
//!
//! — and the gate turns red on the day the date passes, without anybody touching the tree. That
//! is why the check runs on the daily schedule too, and not only on a change.
//!
//! `npm audit` has no ignore list at all, so `deps npm-audit <dir>` runs it for one npm package
//! (production dependencies only: `--omit=dev`) and subtracts the exceptions recorded in
//! `<dir>/npm-audit.toml`, which take the same `{ id, reason }` shape and obey the same rule. A
//! package that declares no runtime dependency passes without a lockfile; one that declares any
//! and has no lockfile fails, because there would be nothing to audit.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// One recorded exception: the advisory it accepts and the reason it is accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exception {
    pub id: String,
    pub reason: String,
}

/// One advisory `npm audit` reports against an installed package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmFinding {
    pub package: String,
    pub severity: String,
    /// The GHSA id, taken from the advisory URL.
    pub advisory: String,
    pub title: String,
}

/// Read the exceptions stored at `path` (`["advisories", "ignore"]` in deny.toml, `["ignore"]` in
/// an npm-audit.toml). A missing key is no exceptions. An entry that is a bare id string — the
/// shape cargo-deny also accepts — comes back with an empty reason, so [`problems`] refuses it.
pub fn parse_exceptions(text: &str, path: &[&str]) -> Result<Vec<Exception>> {
    let doc: toml_edit::DocumentMut = text.parse().context("not valid TOML")?;
    let mut item = doc.as_item();
    for key in path {
        match item.get(key) {
            Some(next) => item = next,
            None => return Ok(Vec::new()),
        }
    }
    let at = path.join(".");
    let Some(entries) = item.as_array() else {
        bail!("`{at}` is not an array");
    };
    entries
        .iter()
        .map(|entry| {
            if let Some(id) = entry.as_str() {
                return Ok(Exception {
                    id: id.to_owned(),
                    reason: String::new(),
                });
            }
            let table = entry
                .as_inline_table()
                .with_context(|| format!("an entry of `{at}` is neither an id nor a table"))?;
            let id = table
                .get("id")
                .and_then(|v| v.as_str())
                .with_context(|| format!("an entry of `{at}` has no `id`"))?;
            let reason = table.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            Ok(Exception {
                id: id.to_owned(),
                reason: reason.to_owned(),
            })
        })
        .collect()
}

/// Everything wrong with a list of exceptions on `today` (`YYYY-MM-DD`). Empty means valid.
pub fn problems(exceptions: &[Exception], today: &str) -> Vec<String> {
    let mut out = Vec::new();
    for e in exceptions {
        let Some(expires) = expiry(&e.reason) else {
            out.push(format!(
                "{}: the reason must start with `expires YYYY-MM-DD:` — got {:?}",
                e.id, e.reason
            ));
            continue;
        };
        if expires < today {
            out.push(format!(
                "{}: the exception expired on {expires} — fix the advisory or decide again and \
                 record a new date",
                e.id
            ));
        }
        if !names_a_board_item(&e.reason) {
            out.push(format!(
                "{}: the reason names no board item (`<prefix>.<suffix>`, e.g. 6j6v.2dn0) that \
                 carries the decision",
                e.id
            ));
        }
    }
    out
}

/// The `YYYY-MM-DD` of a reason that starts `expires YYYY-MM-DD:`, if it is a plausible date.
fn expiry(reason: &str) -> Option<&str> {
    let date = reason.strip_prefix("expires ")?.get(..10)?;
    if reason.get(18..19) != Some(":") {
        return None;
    }
    let b = date.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10)) {
        return None;
    }
    let month: u32 = date[5..7].parse().ok()?;
    let day: u32 = date[8..10].parse().ok()?;
    ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some(date)
}

/// Whether `text` holds a board id: four lowercase alphanumerics, a dot, four more, standing alone.
fn names_a_board_item(text: &str) -> bool {
    let b = text.as_bytes();
    let id_char = |c: u8| c.is_ascii_digit() || c.is_ascii_lowercase();
    (4..b.len().saturating_sub(4)).any(|dot| {
        b[dot] == b'.'
            && b[dot - 4..dot].iter().all(|&c| id_char(c))
            && b[dot + 1..dot + 5].iter().all(|&c| id_char(c))
            && (dot == 4 || !b[dot - 5].is_ascii_alphanumeric())
            && b.get(dot + 5).is_none_or(|c| !c.is_ascii_alphanumeric())
    })
}

/// The advisories in an `npm audit --json` report. A report that carries an `error` (the
/// registry could not be asked) is an error here too: an audit that did not run proves nothing.
pub fn npm_findings(audit_json: &str) -> Result<Vec<NpmFinding>> {
    let report: serde_json::Value =
        serde_json::from_str(audit_json).context("npm audit did not print JSON")?;
    if let Some(error) = report.get("error") {
        bail!("npm audit could not run: {error}");
    }
    let vulnerabilities = report
        .get("vulnerabilities")
        .and_then(|v| v.as_object())
        .context("npm audit's report has no `vulnerabilities` object")?;
    let mut out = Vec::new();
    for (package, entry) in vulnerabilities {
        // A `via` that is a plain string only points at another package of the report; the
        // advisory itself is the object form, listed under the package it is filed against.
        for via in entry["via"].as_array().into_iter().flatten() {
            let Some(url) = via.get("url").and_then(|u| u.as_str()) else {
                continue;
            };
            out.push(NpmFinding {
                package: package.clone(),
                severity: via["severity"].as_str().unwrap_or("unknown").to_owned(),
                advisory: url.rsplit('/').next().unwrap_or(url).to_owned(),
                title: via["title"].as_str().unwrap_or("").to_owned(),
            });
        }
    }
    Ok(out)
}

/// The findings no exception covers, and the exceptions no finding needs any more.
pub fn npm_verdict(
    findings: &[NpmFinding],
    exceptions: &[Exception],
) -> (Vec<NpmFinding>, Vec<String>) {
    let open = findings
        .iter()
        .filter(|f| !exceptions.iter().any(|e| e.id == f.advisory))
        .cloned()
        .collect();
    let stale = exceptions
        .iter()
        .filter(|e| !findings.iter().any(|f| f.advisory == e.id))
        .map(|e| e.id.clone())
        .collect();
    (open, stale)
}

/// Whether a package.json declares anything that is installed alongside the package at run time.
pub fn has_runtime_dependencies(package_json: &str) -> Result<bool> {
    let manifest: serde_json::Value =
        serde_json::from_str(package_json).context("package.json is not JSON")?;
    Ok(["dependencies", "optionalDependencies"].iter().any(|key| {
        manifest
            .get(key)
            .and_then(|v| v.as_object())
            .is_some_and(|deps| !deps.is_empty())
    }))
}

/// `cargo xtask deps exceptions`: the ignore entries of `deny.toml`. Returns how many there are.
pub fn check_exceptions(root: &Path) -> Result<usize> {
    let path = root.join("deny.toml");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let exceptions = parse_exceptions(&text, &["advisories", "ignore"])
        .with_context(|| format!("in {}", path.display()))?;
    refuse_problems(&path, &exceptions)?;
    Ok(exceptions.len())
}

/// `cargo xtask deps npm-audit <dir>`: the production dependencies of one npm package.
pub fn npm_audit(dir: &Path) -> Result<String> {
    let manifest = dir.join("package.json");
    let text = std::fs::read_to_string(&manifest)
        .with_context(|| format!("cannot read {}", manifest.display()))?;
    if !has_runtime_dependencies(&text)? {
        return Ok(format!("{}: no runtime dependencies", dir.display()));
    }
    if !dir.join("package-lock.json").is_file() {
        bail!(
            "{}: declares runtime dependencies but has no package-lock.json — there is nothing \
             to audit",
            dir.display()
        );
    }
    let exceptions_path = dir.join("npm-audit.toml");
    let exceptions = if exceptions_path.is_file() {
        let text = std::fs::read_to_string(&exceptions_path)?;
        let list = parse_exceptions(&text, &["ignore"])
            .with_context(|| format!("in {}", exceptions_path.display()))?;
        refuse_problems(&exceptions_path, &list)?;
        list
    } else {
        Vec::new()
    };
    // npm audit exits non-zero whenever it finds anything, so its status says nothing here; the
    // report decides, and a report that is not one fails in npm_findings.
    let out = Command::new("npm")
        .args(["audit", "--omit=dev", "--json"])
        .current_dir(dir)
        .output()
        .context("cannot run npm")?;
    let findings = npm_findings(&String::from_utf8_lossy(&out.stdout))
        .with_context(|| format!("{}: npm audit", dir.display()))?;
    let (open, stale) = npm_verdict(&findings, &exceptions);
    for id in &stale {
        eprintln!(
            "warning: {}: exception {id} matches no advisory any more — remove it",
            exceptions_path.display()
        );
    }
    if !open.is_empty() {
        let lines: Vec<String> = open
            .iter()
            .map(|f| {
                format!(
                    "  {} {} {} — {}",
                    f.severity, f.package, f.advisory, f.title
                )
            })
            .collect();
        bail!(
            "{}: {} advisory(ies) against production dependencies — fix them, or record each \
             in {} (docs/runbooks/dependency-vulnerabilities.md):\n{}",
            dir.display(),
            open.len(),
            exceptions_path.display(),
            lines.join("\n")
        );
    }
    Ok(format!(
        "{}: npm audit clean ({} finding(s) covered by a recorded exception)",
        dir.display(),
        findings.len()
    ))
}

/// The npm packages that ship, and so are audited: the agent sidecar, whose bundle is compiled
/// into every `nxs`, and the `@nexus-flow/mcp` runner shim.
pub const SHIPPED_NPM_PACKAGES: &[&str] = &["agent-sidecar", "npm/mcp"];

/// The outcome of every check, in order: `Ok` with each line when all passed, `Err` naming every
/// check that failed. Every check has run by then — one red never hides the next.
pub fn verdict(results: Vec<(String, Result<String>)>) -> Result<String> {
    let total = results.len();
    let mut passed = Vec::new();
    let mut failed = Vec::new();
    for (check, result) in results {
        match result {
            Ok(line) => passed.push(format!("{check}: {line}")),
            Err(e) => failed.push(format!("{check}: {e:#}")),
        }
    }
    if failed.is_empty() {
        return Ok(passed.join("\n"));
    }
    bail!(
        "{} of {total} dependency check(s) failed (docs/runbooks/dependency-vulnerabilities.md):\n\n{}",
        failed.len(),
        failed.join("\n\n")
    )
}

/// `cargo xtask deps audit`: the whole gate — `cargo deny check`, the exceptions of deny.toml and
/// npm audit for every shipped npm package. The one command both CI and the daily run invoke.
pub fn audit(root: &Path) -> Result<String> {
    let mut results = vec![("cargo deny check".to_owned(), cargo_deny(root))];
    results.push((
        "deny.toml exceptions".to_owned(),
        check_exceptions(root).map(|n| format!("{n} recorded, none expired")),
    ));
    for dir in SHIPPED_NPM_PACKAGES {
        results.push((format!("npm audit {dir}"), npm_audit(&root.join(dir))));
    }
    verdict(results)
}

/// `cargo deny check` over deny.toml. Its own report goes to the log as it runs; the status
/// decides.
fn cargo_deny(root: &Path) -> Result<String> {
    let status = Command::new("cargo")
        .args(["deny", "check"])
        .current_dir(root)
        .status()
        .context("cannot run `cargo deny` — install cargo-deny")?;
    if !status.success() {
        bail!("refused ({status}); its report is above");
    }
    Ok("advisories, bans, licenses and sources ok".to_owned())
}

fn refuse_problems(path: &Path, exceptions: &[Exception]) -> Result<()> {
    let found = problems(exceptions, &crate::changelog::today());
    if found.is_empty() {
        return Ok(());
    }
    bail!(
        "{}: {} exception(s) not valid (docs/runbooks/dependency-vulnerabilities.md):\n  {}",
        path.display(),
        found.len(),
        found.join("\n  ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(id: &str, reason: &str) -> Exception {
        Exception {
            id: id.into(),
            reason: reason.into(),
        }
    }

    const VALID: &str = "expires 2027-01-31: no maintained successor yet (6j6v.abcd)";

    #[test]
    fn deny_toml_ignore_entries_are_read_with_their_reasons() {
        let text = r#"
[advisories]
version = 2
ignore = [
    { id = "RUSTSEC-2024-0449", reason = "expires 2027-01-31: why (6j6v.abcd)" },
    "RUSTSEC-2025-0001",
]
"#;
        let got = parse_exceptions(text, &["advisories", "ignore"]).unwrap();
        assert_eq!(
            got,
            vec![
                ex("RUSTSEC-2024-0449", "expires 2027-01-31: why (6j6v.abcd)"),
                ex("RUSTSEC-2025-0001", ""),
            ]
        );
    }

    #[test]
    fn a_file_without_the_key_has_no_exceptions() {
        assert!(
            parse_exceptions("[advisories]\nversion = 2\n", &["advisories", "ignore"])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn an_entry_without_an_id_is_a_parse_error() {
        let text = "ignore = [{ reason = \"expires 2027-01-31: x (6j6v.abcd)\" }]\n";
        assert!(parse_exceptions(text, &["ignore"]).is_err());
    }

    #[test]
    fn a_dated_exception_naming_its_item_is_valid_up_to_and_on_its_expiry_day() {
        let list = [ex("RUSTSEC-2024-0449", VALID)];
        assert!(problems(&list, "2026-10-24").is_empty());
        assert!(problems(&list, "2027-01-31").is_empty());
    }

    #[test]
    fn an_exception_is_refused_the_day_after_it_expires() {
        let got = problems(&[ex("RUSTSEC-2024-0449", VALID)], "2027-02-01");
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(
            got[0].contains("RUSTSEC-2024-0449") && got[0].contains("expired"),
            "{got:?}"
        );
    }

    #[test]
    fn an_exception_without_an_expiry_at_the_head_of_its_reason_is_refused() {
        for reason in [
            "",
            "no maintained successor yet (6j6v.abcd)",
            "no successor, expires 2027-01-31 (6j6v.abcd)",
            "expires 2027-1-31: short month (6j6v.abcd)",
            "expires 2027-13-01: no such month (6j6v.abcd)",
            "expires 2027-01-31 no colon (6j6v.abcd)",
        ] {
            let got = problems(&[ex("RUSTSEC-2024-0449", reason)], "2026-10-24");
            assert_eq!(got.len(), 1, "reason {reason:?} must be refused: {got:?}");
            assert!(got[0].contains("RUSTSEC-2024-0449"), "{got:?}");
        }
    }

    #[test]
    fn an_exception_that_names_no_board_item_is_refused() {
        let got = problems(
            &[ex(
                "RUSTSEC-2024-0449",
                "expires 2027-01-31: accepted for now",
            )],
            "2026-10-24",
        );
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(got[0].contains("board item"), "{got:?}");
    }

    /// Trimmed from a real `npm audit --omit=dev --json` run on agent-sidecar before 2dn0 fixed
    /// it: a package with its own advisory, and one whose `via` only points at another package.
    const AUDIT: &str = r#"{
  "auditReportVersion": 2,
  "vulnerabilities": {
    "proxy-addr": {
      "name": "proxy-addr", "severity": "critical", "isDirect": false,
      "via": [{ "source": 1, "name": "proxy-addr", "title": "proxy-addr vulnerable to IP spoofing",
                "url": "https://github.com/advisories/GHSA-jqcg-44mw-7w3h", "severity": "critical" }],
      "range": "1.1.0 - 2.0.7", "nodes": ["node_modules/proxy-addr"], "fixAvailable": true
    },
    "express": {
      "name": "express", "severity": "critical", "isDirect": false, "via": ["proxy-addr"],
      "range": "5.0.0 - 5.2.1", "nodes": ["node_modules/express"], "fixAvailable": true
    }
  },
  "metadata": { "vulnerabilities": { "critical": 2, "total": 2 } }
}"#;

    #[test]
    fn npm_findings_are_the_advisories_not_the_packages_that_merely_pull_them_in() {
        let got = npm_findings(AUDIT).unwrap();
        assert_eq!(
            got,
            vec![NpmFinding {
                package: "proxy-addr".into(),
                severity: "critical".into(),
                advisory: "GHSA-jqcg-44mw-7w3h".into(),
                title: "proxy-addr vulnerable to IP spoofing".into(),
            }]
        );
    }

    #[test]
    fn a_clean_npm_report_has_no_findings() {
        let clean = r#"{"auditReportVersion":2,"vulnerabilities":{},"metadata":{}}"#;
        assert!(npm_findings(clean).unwrap().is_empty());
    }

    #[test]
    fn an_npm_report_that_carries_an_error_fails_closed() {
        let failed = r#"{"error":{"code":"ENOTFOUND","summary":"request failed"}}"#;
        assert!(npm_findings(failed).is_err());
        assert!(npm_findings("not json").is_err());
    }

    #[test]
    fn an_exception_covers_its_advisory_and_an_unneeded_one_is_reported_stale() {
        let findings = npm_findings(AUDIT).unwrap();
        let (open, stale) = npm_verdict(&findings, &[]);
        assert_eq!(open, findings, "without an exception every finding is open");
        assert!(stale.is_empty());

        let (open, stale) = npm_verdict(
            &findings,
            &[
                ex("GHSA-jqcg-44mw-7w3h", VALID),
                ex("GHSA-xxxx-xxxx-xxxx", VALID),
            ],
        );
        assert!(open.is_empty(), "{open:?}");
        assert_eq!(stale, vec!["GHSA-xxxx-xxxx-xxxx".to_string()]);
    }

    #[test]
    fn runtime_dependencies_are_dependencies_or_optional_ones_never_dev() {
        assert!(!has_runtime_dependencies(r#"{"name":"x"}"#).unwrap());
        assert!(
            !has_runtime_dependencies(r#"{"dependencies":{},"devDependencies":{"a":"1"}}"#)
                .unwrap()
        );
        assert!(has_runtime_dependencies(r#"{"dependencies":{"a":"1"}}"#).unwrap());
        assert!(has_runtime_dependencies(r#"{"optionalDependencies":{"a":"1"}}"#).unwrap());
    }
    #[test]
    fn the_verdict_passes_only_when_every_check_passed_and_says_what_each_found() {
        let all_green = verdict(vec![
            ("cargo deny check".into(), Ok("advisories ok".into())),
            ("agent-sidecar".into(), Ok("npm audit clean".into())),
        ])
        .unwrap();
        assert!(
            all_green.contains("cargo deny check: advisories ok"),
            "{all_green}"
        );
        assert!(
            all_green.contains("agent-sidecar: npm audit clean"),
            "{all_green}"
        );
    }

    #[test]
    fn the_verdict_names_every_failed_check_not_only_the_first() {
        let err = verdict(vec![
            (
                "cargo deny check".into(),
                Err(anyhow::anyhow!("advisories FAILED")),
            ),
            ("deny.toml exceptions".into(), Ok("1 recorded".into())),
            ("agent-sidecar".into(), Err(anyhow::anyhow!("2 advisories"))),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("2 of 3"), "{err}");
        assert!(
            err.contains("cargo deny check") && err.contains("advisories FAILED"),
            "{err}"
        );
        assert!(
            err.contains("agent-sidecar") && err.contains("2 advisories"),
            "{err}"
        );
    }
}
