//! The rules on what comes back to the owner, read from GENESIS.md (RFC 0010, "The rules from
//! GENESIS"; RFC 0004, "The rules on what comes back to the owner").
//!
//! The rules are the CommonMark list that starts on the first non-blank line after the mark, a line
//! that reads exactly `<!-- nexus-flow: owner-rules -->` apart from surrounding whitespace. Each
//! top-level item of that list is one rule, with the effect `owner` and the id `genesis.<n>`.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use std::ops::Range;

use crate::markdown::{fences, mark_lines, next_non_blank_line};

/// The line that marks the list of rules.
pub const MARK: &str = "<!-- nexus-flow: owner-rules -->";

/// One rule from GENESIS.md: its id, `genesis.<n>` for its 1-based position in the list, and its
/// text as written, without the top-level item's list marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenesisRule {
    pub id: String,
    pub text: String,
}

/// Why GENESIS.md has no readable rules. Each one sends every pull request to the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenesisError {
    /// The file does not exist at the base commit.
    FileMissing,
    /// No line outside a fenced code block is the mark.
    MarkMissing,
    /// More than one line outside a fenced code block is the mark.
    MarkDoubled,
    /// The first non-blank line after the mark does not start a list.
    ListMissing,
}

impl GenesisError {
    /// The error's code in `nxs merge-gate rules` (RFC 0010).
    pub fn code(self) -> &'static str {
        match self {
            GenesisError::FileMissing => "file_missing",
            GenesisError::MarkMissing => "mark_missing",
            GenesisError::MarkDoubled => "mark_doubled",
            GenesisError::ListMissing => "list_missing",
        }
    }
}

/// The rules in `genesis`, the text of GENESIS.md, or `None` when the file is missing.
pub fn read_genesis_rules(genesis: Option<&str>) -> Result<Vec<GenesisRule>, GenesisError> {
    let source = genesis.ok_or(GenesisError::FileMissing)?;
    let events: Vec<(Event, Range<usize>)> = Parser::new(source).into_offset_iter().collect();
    let after_mark = mark_end(source, &fences(&events))?;
    let list_line = next_non_blank_line(source, after_mark).ok_or(GenesisError::ListMissing)?;
    let items = top_level_items(&events, list_line).ok_or(GenesisError::ListMissing)?;
    Ok(items
        .into_iter()
        .enumerate()
        .map(|(i, range)| GenesisRule {
            id: format!("genesis.{}", i + 1),
            text: item_text(&source[range]),
        })
        .collect())
}

/// The offset just after the line that is the mark, the one such line outside `fences`.
fn mark_end(source: &str, fences: &[Range<usize>]) -> Result<usize, GenesisError> {
    let mut marks = mark_lines(source, fences, MARK);
    let mark = marks.next().ok_or(GenesisError::MarkMissing)?;
    match marks.next() {
        Some(_) => Err(GenesisError::MarkDoubled),
        None => Ok(mark.end),
    }
}

/// The ranges of the top-level items of the list that starts on `line`, each from its marker to
/// the next top-level item or the end of the list, so that nested items stay with their parent.
fn top_level_items(
    events: &[(Event, Range<usize>)],
    line: Range<usize>,
) -> Option<Vec<Range<usize>>> {
    let first = events.iter().position(|(event, range)| {
        matches!(event, Event::Start(Tag::List(_))) && line.contains(&range.start)
    })?;
    let list_end = events[first].1.end;
    let mut depth = 0;
    let mut starts = Vec::new();
    for (event, range) in &events[first..] {
        match event {
            Event::Start(Tag::List(_)) => depth += 1,
            Event::End(TagEnd::List(_)) if depth == 1 => break,
            Event::End(TagEnd::List(_)) => depth -= 1,
            Event::Start(Tag::Item) if depth == 1 => starts.push(range.start),
            _ => {}
        }
    }
    let ends = starts.iter().skip(1).copied().chain([list_end]);
    Some(
        starts
            .iter()
            .copied()
            .zip(ends)
            .map(|(s, e)| s..e)
            .collect(),
    )
}

/// An item's text: its source without the list marker, a bullet or a number with its `.` or `)`,
/// and without the whitespace after the marker and at the end.
fn item_text(item: &str) -> String {
    let rest = match item.strip_prefix(['-', '*', '+']) {
        Some(rest) => rest,
        None => item
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .strip_prefix(['.', ')'])
            .unwrap_or(item),
    };
    rest.trim_start_matches([' ', '\t']).trim_end().to_string()
}
