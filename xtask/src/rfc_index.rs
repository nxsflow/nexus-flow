//! The index of the RFCs in `docs/rfc/` (NEX-85).
//!
//! Every RFC pull request used to add a row to a hand-kept table in `docs/rfc/README.md`, so each
//! merge left the other open RFC pull requests in conflict, and the fix dismissed the owner's
//! approval. No table is stored any more: `cargo xtask rfc-index` prints the index from the RFC
//! files, each `NNNN-<slug>.md` with its `# RFC NNNN: Title` heading and its `Status:` line, and
//! `--md` prints it as a Markdown table. An RFC pull request touches no shared file. `--check` is
//! the gate: it fails when two RFC files share a number or a heading is wrong, never on staleness.

use std::path::Path;

use anyhow::{bail, Context, Result};

/// One RFC file, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rfc {
    pub number: String,
    pub file: String,
    pub title: String,
    pub status: String,
}

/// Every RFC file in `dir`, by number. The template (`0000-template.md`) and README.md are not RFCs.
pub fn read_rfcs(dir: &Path) -> Result<Vec<Rfc>> {
    let mut rfcs = Vec::new();
    let entries = std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries {
        let file = entry?.file_name().to_string_lossy().into_owned();
        let Some(number) = rfc_number(&file) else {
            continue;
        };
        let text =
            std::fs::read_to_string(dir.join(&file)).with_context(|| format!("reading {file}"))?;
        rfcs.push(parse(&file, number, &text)?);
    }
    rfcs.sort_by(|a, b| (&a.number, &a.file).cmp(&(&b.number, &b.file)));
    if let Some(pair) = rfcs.windows(2).find(|w| w[0].number == w[1].number) {
        bail!(
            "{} and {} share the number {}: give the newer one the next free number",
            pair[0].file,
            pair[1].file,
            pair[0].number
        );
    }
    Ok(rfcs)
}

/// `NNNN` of a file named `NNNN-<slug>.md`, unless it is the template's `0000`.
fn rfc_number(file: &str) -> Option<&str> {
    let stem = file.strip_suffix(".md")?;
    let (number, slug) = stem.split_once('-')?;
    let well_formed = number.len() == 4 && number.bytes().all(|b| b.is_ascii_digit());
    (well_formed && !slug.is_empty() && number != "0000").then_some(number)
}

/// The title from the first line, `# RFC NNNN: Title`, and the first `Status:` line.
fn parse(file: &str, number: &str, text: &str) -> Result<Rfc> {
    let heading = text.lines().next().unwrap_or_default();
    let title = heading
        .strip_prefix(&format!("# RFC {number}: "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .with_context(|| {
            format!("{file}: the first line must be `# RFC {number}: <title>`, found `{heading}`")
        })?;
    let status = text
        .lines()
        .find_map(|l| l.strip_prefix("Status:"))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .with_context(|| format!("{file}: no `Status:` line"))?;
    Ok(Rfc {
        number: number.to_string(),
        file: file.to_string(),
        title: title.to_string(),
        status: status.to_string(),
    })
}

/// The Markdown table for `rfcs`, header included, one row per RFC.
pub fn render(rfcs: &[Rfc]) -> String {
    let cell = |text: &str| text.replace('|', "\\|");
    let mut table = String::from("| RFC | Title | Status |\n| --- | --- | --- |\n");
    for rfc in rfcs {
        table.push_str(&format!(
            "| [{}]({}) | {} | {} |\n",
            rfc.number,
            rfc.file,
            cell(&rfc.title),
            cell(&rfc.status)
        ));
    }
    table
}

/// The index of the RFCs in `dir`: one `NNNN · Title · Status` line each, or with `md` a
/// Markdown table.
pub fn index(dir: &Path, md: bool) -> Result<String> {
    let rfcs = read_rfcs(dir)?;
    if md {
        return Ok(render(&rfcs));
    }
    Ok(rfcs
        .iter()
        .map(|r| format!("{} · {} · {}\n", r.number, r.title, r.status))
        .collect())
}

/// The gate: fail when two RFC files share a number, or a file's heading or `Status:` line is
/// wrong. It reads only the RFC files, so it fails on a mistake in them, never on a stale table.
pub fn check(dir: &Path) -> Result<()> {
    read_rfcs(dir).map(|_| ())
}

#[cfg(test)]
#[path = "rfc_index_tests.rs"]
mod tests;
