//! The CODEOWNERS report (RFC 0006, "CODEOWNERS is reported, never written"; RFC 0004): which of
//! the factory's paths no rule in the repository's CODEOWNERS file covers. Nothing here writes.
//!
//! The file is the first of `.github/CODEOWNERS`, `CODEOWNERS` and `docs/CODEOWNERS` that exists,
//! the places and order GitHub looks in. A path is covered when the last rule matching it names at
//! least one owner, as GitHub reads the file: later rules win, and a rule without owners leaves
//! its paths without a code owner.
//!
//! The patterns are GitHub's, which follow `.gitignore` with its documented differences ("About
//! code owners"):
//!
//! - `*` matches any run of characters but `/`, and `?` one character but `/`.
//! - A pattern with a `/` at its start or in its middle is anchored at the root; any other
//!   matches at any depth.
//! - `**` as a whole segment matches any number of directories: zero or more at the start or in
//!   the middle, one or more at the end.
//! - A pattern matches a path, or a directory the path is in; a trailing `/` matches directories
//!   only, and a pattern ending in the segment `*` matches the files directly in its directory
//!   only (`docs/*` owns `docs/a.md`, not `docs/b/c.md`).
//! - Paths are case sensitive.
//! - A line GitHub cannot read is skipped: negation with `!` and character ranges with `[ ]` are
//!   not supported.

use std::fs;
use std::io;
use std::path::Path;

/// Where GitHub looks for CODEOWNERS, in the order it looks.
pub const LOCATIONS: [&str; 3] = [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"];

/// A pattern of GitHub's CODEOWNERS, cut into segments.
#[derive(Debug, Clone)]
struct Pattern {
    /// The segments to match from the root; an unanchored pattern starts with `**`.
    segments: Vec<String>,
    /// The pattern ended in `/`: it matches directories only.
    directory_only: bool,
    /// The pattern's last segment is `*`: it matches no directory, so nothing beneath one.
    files_only: bool,
}

impl Pattern {
    /// The pattern of a rule, or `None` for one GitHub skips.
    fn new(text: &str) -> Option<Pattern> {
        if text.starts_with('!') || text.contains(['[', ']']) {
            return None;
        }
        let directory_only = text.ends_with('/');
        let trimmed = text.strip_suffix('/').unwrap_or(text);
        let anchored = trimmed.contains('/');
        let trimmed = trimmed.strip_prefix('/').unwrap_or(trimmed);
        if trimmed.is_empty() {
            return None;
        }
        let mut segments: Vec<String> = Vec::new();
        if !anchored {
            segments.push("**".to_string());
        }
        for segment in trimmed.split('/') {
            if segment.is_empty() {
                return None;
            }
            segments.push(segment.to_string());
        }
        let files_only = segments.last().is_some_and(|s| s == "*");
        Some(Pattern {
            segments,
            directory_only,
            files_only,
        })
    }

    /// Whether the pattern matches `path`, or a directory `path` is in.
    fn matches(&self, path: &str) -> bool {
        let parts: Vec<&str> = path.split('/').collect();
        (1..=parts.len()).any(|end| {
            let whole = end == parts.len();
            if (whole && self.directory_only) || (!whole && self.files_only) {
                return false;
            }
            segments_match(&self.segments, &parts[..end])
        })
    }
}

/// Whether the pattern's segments match the path's segments, all of them.
fn segments_match(pattern: &[String], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((first, [])) if first == "**" => !path.is_empty(),
        Some((first, rest)) if first == "**" => {
            (0..=path.len()).any(|skip| segments_match(rest, &path[skip..]))
        }
        Some((first, rest)) => match path.split_first() {
            Some((part, path)) => {
                glob(first.as_bytes(), part.as_bytes()) && segments_match(rest, path)
            }
            None => false,
        },
    }
}

/// Whether one segment's glob, with `*` and `?`, matches one path segment.
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => (0..=text.len()).any(|skip| glob(rest, &text[skip..])),
        Some((b'?', rest)) => !text.is_empty() && glob(rest, char_tail(text)),
        Some((&c, rest)) => text.first() == Some(&c) && glob(rest, &text[1..]),
    }
}

/// `text` after its first UTF-8 character, so `?` matches a character, not a byte.
fn char_tail(text: &[u8]) -> &[u8] {
    let len = match text[0] {
        b if b < 0x80 => 1,
        b if b >= 0xF0 => 4,
        b if b >= 0xE0 => 3,
        _ => 2,
    };
    &text[len.min(text.len())..]
}

/// One readable line of CODEOWNERS: its pattern and its owners, in the order written.
#[derive(Debug, Clone)]
struct Rule {
    pattern: Pattern,
    owners: Vec<String>,
}

/// The rules of a CODEOWNERS file, in file order.
#[derive(Debug, Clone)]
pub struct Codeowners {
    rules: Vec<Rule>,
}

impl Codeowners {
    /// Reads the text of a CODEOWNERS file. A comment, a blank line, and a line GitHub cannot read
    /// are no rules; everything after a `#` that starts a word is a comment.
    pub fn parse(text: &str) -> Codeowners {
        let rules = text
            .lines()
            .filter_map(|line| {
                let mut words = line.split_whitespace().take_while(|w| !w.starts_with('#'));
                let pattern = Pattern::new(words.next()?)?;
                let owners = words.map(str::to_string).collect();
                Some(Rule { pattern, owners })
            })
            .collect();
        Codeowners { rules }
    }

    /// The owners of the last rule that matches `path`, or `None` when no rule does. An empty
    /// list means the last matching rule names no owner.
    pub fn owners(&self, path: &str) -> Option<&[String]> {
        self.rules
            .iter()
            .rev()
            .find(|rule| rule.pattern.matches(path))
            .map(|rule| rule.owners.as_slice())
    }

    /// Whether a rule gives `path` a code owner.
    pub fn covers(&self, path: &str) -> bool {
        self.owners(path).is_some_and(|owners| !owners.is_empty())
    }
}

/// The first CODEOWNERS file `read` finds, in GitHub's order, with its location and text.
/// `read` answers the text at a location relative to the repository's root, or `None` when there
/// is no file there; the merge gate reads a commit, `install` the working tree.
pub fn find(
    mut read: impl FnMut(&str) -> io::Result<Option<String>>,
) -> io::Result<Option<(&'static str, String)>> {
    for location in LOCATIONS {
        if let Some(text) = read(location)? {
            return Ok(Some((location, text)));
        }
    }
    Ok(None)
}

/// The first CODEOWNERS file in the working tree at `root`, in GitHub's order. Only a regular file
/// whose name is exactly `CODEOWNERS` counts, also on a file system that ignores case.
pub fn find_in(root: &Path) -> io::Result<Option<(&'static str, String)>> {
    find(|location| {
        let path = root.join(location);
        let dir = path.parent().unwrap_or(root);
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(None)
            }
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            if entry.file_name() == "CODEOWNERS" && entry.file_type()?.is_file() {
                return fs::read_to_string(entry.path()).map(Some);
            }
        }
        Ok(None)
    })
}

/// The `codeowners` part of `install`'s report: the file found, and the paths no rule covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The CODEOWNERS file's location relative to the root, such as `.github/CODEOWNERS`.
    pub file: String,
    /// The paths no rule covers, in the order they were given.
    pub uncovered: Vec<String>,
}

/// Which of `paths` the CODEOWNERS file in the working tree at `root` leaves without a code owner,
/// or `None` when the repository has no CODEOWNERS file. It only reads.
pub fn report(root: &Path, paths: &[&str]) -> io::Result<Option<Report>> {
    let Some((file, text)) = find_in(root)? else {
        return Ok(None);
    };
    let owners = Codeowners::parse(&text);
    Ok(Some(Report {
        file: file.to_string(),
        uncovered: paths
            .iter()
            .filter(|path| !owners.covers(path))
            .map(|path| path.to_string())
            .collect(),
    }))
}
