//! What the gate's two marked Markdown texts share, GENESIS.md and the verdict's comment: lines,
//! fenced code blocks, and the lines that are a mark outside them.

use pulldown_cmark::{CodeBlockKind, Event, Tag};
use std::ops::Range;

/// The byte ranges of the fenced code blocks, their fences included.
pub(crate) fn fences(events: &[(Event, Range<usize>)]) -> Vec<Range<usize>> {
    events
        .iter()
        .filter(|(event, _)| {
            matches!(
                event,
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_)))
            )
        })
        .map(|(_, range)| range.clone())
        .collect()
}

/// The lines that read exactly `mark`, apart from surrounding whitespace, outside `fences`.
pub(crate) fn mark_lines<'a>(
    source: &'a str,
    fences: &'a [Range<usize>],
    mark: &'a str,
) -> impl Iterator<Item = Range<usize>> + 'a {
    lines(source).filter(move |line| {
        source[line.clone()].trim() == mark && !fences.iter().any(|f| f.contains(&line.start))
    })
}

/// The first line from `from` on that holds more than whitespace.
pub(crate) fn next_non_blank_line(source: &str, from: usize) -> Option<Range<usize>> {
    lines(&source[from..])
        .map(|line| line.start + from..line.end + from)
        .find(|line| !source[line.clone()].trim().is_empty())
}

/// The byte range of each line, its line ending included.
fn lines(source: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    source.split_inclusive('\n').scan(0, |start, line| {
        let range = *start..*start + line.len();
        *start = range.end;
        Some(range)
    })
}
