//! The decision log of the software factory (`factory/FACTORY.md`, "The decision log"): one
//! Markdown file per decision in `factory/decisions/`, named `YYYY-MM-DD-<ticket>-<slug>.md`, so
//! two pull requests never edit the same file. `cargo xtask decisions` reads the folder and prints
//! it; `cargo xtask decisions --check` checks every file and is the gate.
//!
//! A port of nocentraldb's `scripts/decisions.mjs`, so both repositories read and check their logs
//! by the same rules: the name pattern, the heading's date and first ticket against the file
//! name, the four required fields, lowercase topics, and that each `Supersedes:` file exists.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};

/// The fields every decision must carry, in the order the template lists them.
const REQUIRED: [&str; 4] = ["Decided by", "Why", "Instead of", "Topics"];

/// What to print: the whole log, or a filtered part of it, in full or as an index.
#[derive(Debug, Default, Clone)]
pub struct Filter {
    pub index: bool,
    pub current: bool,
    pub ticket: Option<String>,
    pub topic: Option<String>,
    pub since: Option<String>,
}

/// One decision file, parsed.
#[derive(Debug, Clone)]
pub struct Entry {
    pub file: String,
    pub date: String,
    /// The heading's ticket part as written: `no ticket`, `ABC-9, ABC-10` or `ABC-6 to ABC-13`.
    pub ticket: String,
    /// The ticket ids that part names.
    pub tickets: Vec<String>,
    pub title: String,
    pub topics: Vec<String>,
    pub supersedes: Vec<String>,
    pub text: String,
}

/// `YYYY-MM-DD` and a real calendar date.
pub fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<u32> {
        let s = &text[r];
        s.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| s.parse().ok())
            .flatten()
    };
    let (Some(y), Some(m), Some(d)) = (digits(0..4), digits(5..7), digits(8..10)) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

/// A ticket id like `ABC-22`: an uppercase letter, uppercase letters or digits, `-`, digits.
fn is_ticket(text: &str) -> bool {
    split_ticket(text).is_some()
}

/// `ABC-22` into (`ABC`, 22).
fn split_ticket(text: &str) -> Option<(&str, u64)> {
    let (prefix, number) = text.rsplit_once('-')?;
    let mut chars = prefix.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_uppercase());
    let rest_ok = chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    let number_ok = !number.is_empty() && number.bytes().all(|c| c.is_ascii_digit());
    (first_ok && rest_ok && number_ok).then(|| Some((prefix, number.parse().ok()?)))?
}

/// A slug: lowercase letters and digits in words joined by single `-`.
fn is_slug(text: &str) -> bool {
    !text.is_empty()
        && text.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// A topic: starts with a lowercase letter or digit, then lowercase letters, digits or `-`.
fn is_topic(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && bytes.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|item| item.trim().to_string())
        .collect()
}

/// The ticket ids a heading's ticket part names: `no ticket` names none, `ABC-9, ABC-10` two,
/// `ABC-6 to ABC-13` the range. `None` when it is neither.
pub fn tickets_of(text: &str) -> Option<Vec<String>> {
    if text == "no ticket" {
        return Some(Vec::new());
    }
    let mut ids = Vec::new();
    for part in list(text) {
        if let Some((from, to)) = part.split_once(" to ") {
            let (prefix, a) = split_ticket(from)?;
            let (prefix_to, b) = split_ticket(to)?;
            if prefix != prefix_to || a > b {
                return None;
            }
            ids.extend((a..=b).map(|n| format!("{prefix}-{n}")));
        } else if is_ticket(&part) {
            ids.push(part);
        } else {
            return None;
        }
    }
    Some(ids)
}

/// The file name's (date, ticket) when it matches `YYYY-MM-DD-<ticket>-<slug>.md`.
fn parse_file_name(file: &str) -> Option<(String, String)> {
    let stem = file.strip_suffix(".md")?;
    let date = stem.get(..10)?;
    if !is_date(date) || stem.as_bytes().get(10) != Some(&b'-') {
        return None;
    }
    let rest = &stem[11..];
    let (ticket, slug) = if let Some(slug) = rest.strip_prefix("no-ticket-") {
        ("no-ticket".to_string(), slug)
    } else {
        // The ticket is `<PREFIX>-<digits>`; the slug is everything after the next `-`.
        let (prefix, after) = rest.split_once('-')?;
        let (number, slug) = after.split_once('-')?;
        let ticket = format!("{prefix}-{number}");
        if !is_ticket(&ticket) {
            return None;
        }
        (ticket, slug)
    };
    is_slug(slug).then(|| (date.to_string(), ticket))
}

/// `# YYYY-MM-DD · <ticket> · <title>` into its three parts.
fn parse_heading(line: &str) -> Option<(String, String, String)> {
    let rest = line.strip_prefix("# ")?;
    let (date, rest) = rest.split_once(" · ")?;
    let (ticket, title) = rest.split_once(" · ")?;
    (date.len() == 10 && !ticket.is_empty() && !title.is_empty())
        .then(|| (date.to_string(), ticket.to_string(), title.to_string()))
}

/// A field line `- Name: value`: the name is a capitalised word followed by lowercase words.
fn parse_field(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("- ")?;
    let (name, value) = rest.split_once(':')?;
    let mut words = name.split(' ');
    let first = words.next()?;
    let mut first_chars = first.chars();
    let first_ok = first_chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && first_chars.all(|c| c.is_ascii_lowercase());
    let rest_ok = words.all(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase()));
    (first_ok && rest_ok).then_some((name, value.trim()))
}

/// One decision file, parsed, with the problems the gate reports for it.
fn parse(file: &str, text: &str) -> (Entry, Vec<String>) {
    let mut problems = Vec::new();
    let name = parse_file_name(file);
    if name.is_none() {
        problems.push(
            "the file name does not match YYYY-MM-DD-<ticket>-<slug>.md (<ticket> is no-ticket or \
             an id like ABC-22, <slug> lowercase words joined by -)"
                .to_string(),
        );
    }
    let first_line = text.lines().next().unwrap_or("");
    let heading = parse_heading(first_line);
    let tickets = heading.as_ref().and_then(|(_, t, _)| tickets_of(t));
    match (&heading, &tickets) {
        (Some((date, _, _)), Some(ids)) if is_date(date) => {
            if let Some((name_date, name_ticket)) = &name {
                let file_ticket = ids.first().map_or("no-ticket", String::as_str);
                if date != name_date {
                    problems.push(format!(
                        "the heading's date {date} differs from the file name's {name_date}"
                    ));
                }
                if file_ticket != name_ticket {
                    problems.push(format!(
                        "the heading's first ticket {file_ticket} differs from the file name's \
                         {name_ticket}"
                    ));
                }
            }
        }
        _ => problems.push(
            "the first line is not a heading `# YYYY-MM-DD · <ticket> · <title>` (<ticket> is \
             `no ticket`, ids like `ABC-9, ABC-10`, or a range `ABC-6 to ABC-13`)"
                .to_string(),
        ),
    }
    let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
    for line in text.lines().skip(1) {
        if let Some((key, value)) = parse_field(line) {
            fields.entry(key).or_insert(value);
        }
    }
    for key in REQUIRED {
        if fields.get(key).is_none_or(|v| v.is_empty()) {
            problems.push(format!("the field `- {key}:` is missing or empty"));
        }
    }
    let topics = fields
        .get("Topics")
        .filter(|v| !v.is_empty())
        .map(|v| list(v))
        .unwrap_or_default();
    if topics.iter().any(|t| !is_topic(t)) {
        problems.push(
            "Topics must be lowercase tags (letters, digits, -) separated by commas".to_string(),
        );
    }
    let supersedes = fields
        .get("Supersedes")
        .map(|v| list(v))
        .unwrap_or_default();
    let (date, ticket, title) = match heading {
        Some((d, t, title)) => (d, t, title),
        None => (
            name.map(|(d, _)| d).unwrap_or_default(),
            String::new(),
            String::new(),
        ),
    };
    let entry = Entry {
        file: file.to_string(),
        date,
        ticket,
        tickets: tickets.unwrap_or_default(),
        title,
        topics,
        supersedes,
        text: text.to_string(),
    };
    (entry, problems)
}

/// Every decision in `dir`, sorted by file name (date order, same day by name), and every problem
/// the gate reports, as `<file>: <problem>`. A missing folder is an empty log.
pub fn read_decisions(dir: &Path) -> Result<(Vec<Entry>, Vec<String>)> {
    let mut files: Vec<String> = Vec::new();
    if dir.exists() {
        for item in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let item = item?;
            if item.file_type()?.is_file() {
                files.push(item.file_name().to_string_lossy().into_owned());
            }
        }
    }
    files.sort();
    let names: BTreeSet<&str> = files.iter().map(String::as_str).collect();
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(dir.join(file))
            .with_context(|| format!("reading {}", dir.join(file).display()))?;
        let (entry, mut found) = parse(file, &text);
        for old in &entry.supersedes {
            if !names.contains(old.as_str()) {
                found.push(format!(
                    "Supersedes names {old}, which is not in the folder"
                ));
            }
        }
        problems.extend(found.into_iter().map(|p| format!("{file}: {p}")));
        entries.push(entry);
    }
    Ok((entries, problems))
}

/// For each superseded file, the files that name it in `Supersedes:`.
fn superseded_by(entries: &[Entry]) -> BTreeMap<&str, Vec<&str>> {
    let mut by: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for entry in entries {
        for old in &entry.supersedes {
            by.entry(old.as_str())
                .or_default()
                .push(entry.file.as_str());
        }
    }
    by
}

/// The decisions printed in full, each with what superseded it; with `index`, one line per
/// decision instead. What superseded a decision is looked up in all `entries`, filtered or not.
pub fn render(entries: &[Entry], filter: &Filter) -> String {
    let by = superseded_by(entries);
    let ticket = filter.ticket.as_ref().map(|t| t.to_uppercase());
    let shown = entries.iter().filter(|e| {
        !(filter.current && by.contains_key(e.file.as_str()))
            && ticket.as_ref().is_none_or(|t| e.tickets.contains(t))
            && filter.topic.as_ref().is_none_or(|t| e.topics.contains(t))
            && filter
                .since
                .as_ref()
                .is_none_or(|s| e.date.as_str() >= s.as_str())
    });
    let blocks: Vec<String> = shown
        .map(|e| {
            let later = by.get(e.file.as_str()).map(|files| files.join(", "));
            if filter.index {
                let line = format!("{} · {} · {}", e.date, e.ticket, e.title);
                match later {
                    Some(files) => format!("{line} · superseded by {files}"),
                    None => line,
                }
            } else {
                let body = e.text.trim();
                match later {
                    Some(files) => format!("{body}\n\n> Superseded by {files}"),
                    None => body.to_string(),
                }
            }
        })
        .collect();
    blocks.join(if filter.index { "\n" } else { "\n\n" })
}

/// `cargo xtask decisions`: print the log (filtered), or with `check` only check it. Every
/// problem goes to stderr and fails the command, in both modes.
pub fn run(dir: &Path, filter: &Filter, check: bool) -> Result<()> {
    if let Some(since) = &filter.since {
        anyhow::ensure!(
            is_date(since),
            "decisions: --since takes a date as YYYY-MM-DD, not {since}"
        );
    }
    let (entries, problems) = read_decisions(dir)?;
    if !problems.is_empty() {
        for problem in &problems {
            eprintln!("decisions: {problem}");
        }
        anyhow::bail!(
            "decisions: {} problem(s) in {}",
            problems.len(),
            dir.display()
        );
    }
    if check {
        println!("decisions check: ok ({} decision(s))", entries.len());
        return Ok(());
    }
    let output = render(&entries, filter);
    if !output.is_empty() {
        println!("{output}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "# 2026-11-14 · NXF-3 · Tickets live in Linear\n\n\
        - Decided by: Carsten\n- Why: one line\n- Instead of: the board\n- Topics: tracker, factory\n";

    fn folder(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        dir
    }

    #[test]
    fn a_well_formed_file_has_no_problems() {
        let dir = folder(&[("2026-11-14-NXF-3-tickets-live-in-linear.md", GOOD)]);
        let (entries, problems) = read_decisions(dir.path()).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(entries[0].tickets, vec!["NXF-3"]);
        assert_eq!(entries[0].topics, vec!["tracker", "factory"]);
    }

    #[test]
    fn a_missing_folder_is_an_empty_log() {
        let dir = tempfile::tempdir().unwrap();
        let (entries, problems) = read_decisions(&dir.path().join("decisions")).unwrap();
        assert!(entries.is_empty() && problems.is_empty());
    }

    #[test]
    fn the_file_name_must_match_the_pattern() {
        let dir = folder(&[("tickets-live-in-linear.md", GOOD)]);
        let (_, problems) = read_decisions(dir.path()).unwrap();
        assert!(
            problems[0].contains("the file name does not match"),
            "{problems:?}"
        );
    }

    #[test]
    fn the_heading_must_agree_with_the_file_name() {
        let dir = folder(&[("2026-11-13-NXF-4-tickets-live-in-linear.md", GOOD)]);
        let (_, problems) = read_decisions(dir.path()).unwrap();
        assert!(problems
            .iter()
            .any(|p| p.contains("heading's date 2026-11-14")));
        assert!(problems.iter().any(|p| p.contains("first ticket NXF-3")));
    }

    #[test]
    fn every_required_field_is_reported_when_missing() {
        let text = "# 2026-11-14 · no ticket · Something\n\n- Decided by: pm\n";
        let dir = folder(&[("2026-11-14-no-ticket-something.md", text)]);
        let (_, problems) = read_decisions(dir.path()).unwrap();
        for key in ["Why", "Instead of", "Topics"] {
            assert!(
                problems.iter().any(|p| p.contains(&format!("`- {key}:`"))),
                "{key}"
            );
        }
    }

    #[test]
    fn topics_must_be_lowercase_tags() {
        let text = GOOD.replace("tracker, factory", "Tracker");
        let dir = folder(&[("2026-11-14-NXF-3-tickets-live-in-linear.md", &text)]);
        let (_, problems) = read_decisions(dir.path()).unwrap();
        assert!(problems
            .iter()
            .any(|p| p.contains("Topics must be lowercase")));
    }

    #[test]
    fn a_superseded_file_must_exist() {
        let text = format!("{GOOD}- Supersedes: 2026-11-01-no-ticket-gone.md\n");
        let dir = folder(&[("2026-11-14-NXF-3-tickets-live-in-linear.md", &text)]);
        let (_, problems) = read_decisions(dir.path()).unwrap();
        assert!(problems
            .iter()
            .any(|p| p.contains("which is not in the folder")));
    }

    #[test]
    fn ticket_parts_name_lists_and_ranges() {
        assert_eq!(tickets_of("no ticket"), Some(vec![]));
        assert_eq!(
            tickets_of("NXF-9, NXF-10").unwrap(),
            vec!["NXF-9", "NXF-10"]
        );
        assert_eq!(
            tickets_of("NXF-6 to NXF-8").unwrap(),
            vec!["NXF-6", "NXF-7", "NXF-8"]
        );
        assert_eq!(tickets_of("NXF-8 to NXF-6"), None);
        assert_eq!(tickets_of("nxf-1"), None);
    }

    #[test]
    fn dates_are_real_calendar_dates() {
        assert!(is_date("2028-02-29"));
        assert!(!is_date("2026-02-29"));
        assert!(!is_date("2026-13-01"));
        assert!(!is_date("2026-1-01"));
    }

    #[test]
    fn render_filters_and_marks_superseded_decisions() {
        let newer = "# 2026-11-15 · NXF-5 · Tickets live in Linear, team NXF\n\n\
            - Decided by: Carsten\n- Why: w\n- Instead of: i\n- Topics: tracker\n\
            - Supersedes: 2026-11-14-NXF-3-tickets-live-in-linear.md\n";
        let dir = folder(&[
            ("2026-11-14-NXF-3-tickets-live-in-linear.md", GOOD),
            ("2026-11-15-NXF-5-tickets-live-in-linear-team-nxf.md", newer),
        ]);
        let (entries, problems) = read_decisions(dir.path()).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let index = render(
            &entries,
            &Filter {
                index: true,
                ..Filter::default()
            },
        );
        assert_eq!(
            index,
            "2026-11-14 · NXF-3 · Tickets live in Linear · superseded by \
             2026-11-15-NXF-5-tickets-live-in-linear-team-nxf.md\n\
             2026-11-15 · NXF-5 · Tickets live in Linear, team NXF"
        );
        let current = Filter {
            index: true,
            current: true,
            ..Filter::default()
        };
        assert_eq!(
            render(&entries, &current),
            "2026-11-15 · NXF-5 · Tickets live in Linear, team NXF"
        );
        let ticket = Filter {
            ticket: Some("nxf-3".into()),
            ..Filter::default()
        };
        assert!(render(&entries, &ticket)
            .ends_with("> Superseded by 2026-11-15-NXF-5-tickets-live-in-linear-team-nxf.md"));
        let topic = Filter {
            index: true,
            topic: Some("factory".into()),
            ..Filter::default()
        };
        assert!(render(&entries, &topic).starts_with("2026-11-14 · NXF-3"));
        let since = Filter {
            index: true,
            since: Some("2026-11-15".into()),
            ..Filter::default()
        };
        assert!(render(&entries, &since).starts_with("2026-11-15"));
    }
}
