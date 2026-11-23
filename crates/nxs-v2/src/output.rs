//! The output contract (RFC 0001, "Reading and writing"): JSON by default, one compact line, with
//! the fields in the order the record declares them; Markdown with `--md`. Neither holds a time,
//! a path or anything else that is not in the local copy, so the same copy gives the same bytes.

use serde::Serialize;

/// How a command's result is printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Md,
}

/// A command's result, printable in both formats.
pub trait Render: Serialize {
    /// The result as Markdown, ending in a newline.
    fn markdown(&self) -> String;
}

impl Format {
    /// `record` as this format prints it.
    pub fn render<R: Render>(self, record: &R) -> String {
        match self {
            Format::Json => {
                let mut line = serde_json::to_string(record).expect("a record serializes to JSON");
                line.push('\n');
                line
            }
            Format::Md => record.markdown(),
        }
    }
}
