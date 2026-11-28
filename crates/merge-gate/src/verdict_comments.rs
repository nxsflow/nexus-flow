//! The verdict, found in the pull request's comments (RFC 0010, "Where the verdict comes from").
//!
//! The comments are GitHub's "list issue comments" response, all pages as one JSON array, of which
//! the gate reads `id`, `user.login`, `created_at` and `body`. The verdict is in the newest comment,
//! by `created_at` and then by `id`, whose author is a verdict author, compared without regard to
//! ASCII case, and whose body holds the mark: the fenced code block with the info string `json`
//! that starts on the first non-blank line after the mark. When that comment has no such block,
//! there is no verdict; an older comment is never a fallback.

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use serde_json::Value;
use std::ops::Range;

use crate::markdown::{fences, mark_lines, next_non_blank_line};

/// The line that marks the verdict in a comment.
pub const MARK: &str = "<!-- nexus-flow: merge-gate-verdict -->";

/// One comment, as the gate reads it.
struct Comment {
    id: u64,
    /// `None` when the forge gives no user, as for a deleted account.
    login: Option<String>,
    created_at: String,
    body: String,
}

/// The verdict's text from `comments`, read from the newest comment by one of `authors` that holds
/// the mark; else every reason there is none, one line each.
pub fn verdict_from_comments(comments: &[u8], authors: &[String]) -> Result<String, Vec<String>> {
    let comments = read_comments(comments)?;
    let newest = comments
        .iter()
        .filter(|c| is_author(c.login.as_deref(), authors) && holds_mark(&c.body))
        .max_by(|a, b| (&a.created_at, a.id).cmp(&(&b.created_at, b.id)))
        .ok_or_else(|| vec!["no comment by a verdict author holds the mark".to_string()])?;
    json_block_after_mark(&newest.body).ok_or_else(|| {
        vec![format!(
            "comment {} has no fenced json block after the mark",
            newest.id
        )]
    })
}

/// The comments in `bytes`; the whole file is refused when any comment lacks what the gate reads.
fn read_comments(bytes: &[u8]) -> Result<Vec<Comment>, Vec<String>> {
    let not_comments = || vec!["the comments are not a JSON array of comments".to_string()];
    let value: Value = serde_json::from_slice(bytes).map_err(|_| not_comments())?;
    let array = value.as_array().ok_or_else(not_comments)?;
    let mut comments = Vec::with_capacity(array.len());
    let mut problems = Vec::new();
    for (i, item) in array.iter().enumerate() {
        match read_comment(item) {
            Some(comment) => comments.push(comment),
            None => problems.push(format!(
                "comment {} of the comments lacks a numeric id, a user, a created_at in \
                 YYYY-MM-DDTHH:MM:SSZ or a string body",
                i + 1
            )),
        }
    }
    if problems.is_empty() {
        Ok(comments)
    } else {
        Err(problems)
    }
}

fn read_comment(item: &Value) -> Option<Comment> {
    let login = match item.get("user")? {
        Value::Null => None,
        user => Some(user.get("login")?.as_str()?.to_string()),
    };
    let created_at = item.get("created_at")?.as_str()?;
    if !is_utc_timestamp(created_at) {
        return None;
    }
    let body = match item.get("body") {
        None | Some(Value::Null) => String::new(),
        Some(body) => body.as_str()?.to_string(),
    };
    Some(Comment {
        id: item.get("id")?.as_u64()?,
        login,
        created_at: created_at.to_string(),
        body,
    })
}

/// Whether `text` is a time in the one form GitHub gives, `YYYY-MM-DDTHH:MM:SSZ`, whose byte order
/// is its time order.
pub(crate) fn is_utc_timestamp(text: &str) -> bool {
    let shape = b"dddd-dd-ddTdd:dd:ddZ";
    text.len() == shape.len()
        && text.bytes().zip(shape).all(|(b, &s)| match s {
            b'd' => b.is_ascii_digit(),
            s => b == s,
        })
}

fn is_author(login: Option<&str>, authors: &[String]) -> bool {
    login.is_some_and(|login| authors.iter().any(|a| a.eq_ignore_ascii_case(login)))
}

fn holds_mark(body: &str) -> bool {
    mark_end(body, &events(body)).is_some()
}

fn events(body: &str) -> Vec<(Event<'_>, Range<usize>)> {
    Parser::new(body).into_offset_iter().collect()
}

/// The offset just after the first line of `body` that is the mark, outside a fenced code block.
fn mark_end(body: &str, events: &[(Event, Range<usize>)]) -> Option<usize> {
    let fences = fences(events);
    let end = mark_lines(body, &fences, MARK).next()?.end;
    Some(end)
}

/// The content of the fenced `json` block that starts on the first non-blank line after the mark.
fn json_block_after_mark(body: &str) -> Option<String> {
    let events = events(body);
    let line = next_non_blank_line(body, mark_end(body, &events)?)?;
    let start = events.iter().position(|(event, range)| {
        matches!(event, Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) if info.trim() == "json")
            && line.contains(&range.start)
    })?;
    let mut text = String::new();
    for (event, _) in &events[start + 1..] {
        match event {
            Event::Text(t) => text.push_str(t),
            Event::End(TagEnd::CodeBlock) => break,
            _ => {}
        }
    }
    Some(text)
}
