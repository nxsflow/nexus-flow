//! Source resolution for the agent-native, escaping-free long-text input path (Epic 95d).
//!
//! A long text may come from exactly one source: an inline flag value (or a positional), STDIN
//! (the `-` sentinel, 95d.1), a file (`--<name>-file`, 95d.2), or — where the consumer offers it —
//! a JSON object piped as the whole write payload (`--json -`, 95d.3). This module is the single
//! seam that classifies and reads those sources, enforcing the two cross-cutting rules —
//! **exactly one source per named text**, and **at most one STDIN reader per invocation** — once,
//! so every variant and every verb share identical behavior instead of re-implementing it.
//! *Validation* of what was read stays with the write path; this layer only turns sources into
//! the concrete strings that path already takes.
//!
//! **It lives in the foundation, and it is named `text_input` rather than `field_input`, because
//! the second consumer is not flow** (nxf 6j6v.s46h). It was `crates/cli/src/field_input.rs` — a
//! private module of `nxf` — until `nxc send`/`nxc reply` needed exactly the same notation for a
//! MESSAGE BODY, which is not a field of anything. `nexus-chat` depends on this crate and must not
//! depend on flow (spec §4.4), so the one place both can reach is here, beside [`crate::error`],
//! whose types this contract is written in. A second copy was the alternative, and a second copy
//! is how `-` comes to mean one thing in `nxf` and another in `nxc`.
//!
//! Nothing here is about escaping BETTER. On both non-inline paths the text passes no shell
//! quoting at all, which is the whole point: `nxc reply --thread <id> "<a review verdict>"` lets
//! the shell evaluate the backticks and `$(…)` a verdict is full of, before `nxc` sees a byte.

use crate::error::{NxfError, Result};
use std::io::Read;

/// The STDIN sentinel: a flag value (or the right-hand side of `--set field=…`) of exactly `-`
/// means "read this field from STDIN". A file path of `-` is the same sentinel (95d.2).
pub const STDIN_SENTINEL: &str = "-";

/// Reads STDIN at most once per invocation. The first field to ask consumes it; a second ask is
/// the "more than one field wants STDIN" error — so the one-reader rule is enforced structurally
/// by sharing a single `StdinReader` across a call, not by a scattered counter.
#[derive(Default)]
pub struct StdinReader {
    used: bool,
}

impl StdinReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read all of STDIN to a `String`, **verbatim** — no trailing-newline trimming, so what an
    /// agent pipes is exactly what is stored (the conservative 95d.1 contract), capped at
    /// [`MAX_INPUT_BYTES`]. Errors with `validation` if STDIN was already consumed by another field
    /// in this same call, if it exceeds the cap, or if it is not valid UTF-8; `io` if it cannot be
    /// read.
    pub fn take(&mut self) -> Result<String> {
        if self.used {
            return Err(NxfError::validation(
                "more than one field reads from STDIN ('-') in a single call; only one field may \
                 read STDIN per invocation (use --<field>-file or a JSON payload for the rest)",
            ));
        }
        self.used = true;
        read_capped(std::io::stdin().lock(), MAX_INPUT_BYTES, "STDIN")
    }
}

/// Resolve one named text from its (at most one) source — the single seam shared by every
/// variant of Epic 95d, and by `nxc`'s message body (nxf 6j6v.s46h):
///
/// - `inline` — the flag value or positional; `Some("-")` reads it from STDIN, `Some(v)` is the
///   literal value.
/// - `file` — a `--<field>-file` / `--set-file` / `--body-file` path (95d.2); a path of `-` is the
///   STDIN sentinel too, so it counts toward the one-reader rule.
///
/// Supplying BOTH an inline value and a file for the same text is the "more than one source"
/// error (`field` names it — the flow field, or `body`). Files are read verbatim and
/// UTF-8-enforced. Returns `None` only when neither source is given. The JSON-payload source
/// (95d.3) feeds the same downstream write path rather than going through here.
pub fn resolve(
    reader: &mut StdinReader,
    field: &str,
    inline: Option<&str>,
    file: Option<&str>,
) -> Result<Option<String>> {
    match (inline, file) {
        (Some(_), Some(_)) => Err(NxfError::validation(format!(
            "field '{field}' is set from more than one source; provide exactly one of an inline \
             value, a file, or STDIN"
        ))),
        (Some(v), None) | (None, Some(v)) if v == STDIN_SENTINEL => reader.take().map(Some),
        (Some(v), None) => Ok(Some(v.to_string())),
        (None, Some(path)) => read_file(path).map(Some),
        (None, None) => Ok(None),
    }
}

/// Interpret the optional `-` positional that selects "read the whole write payload as one JSON
/// object from STDIN" (95d.3). Only `-` is meaningful; any other value is a usage error. Kept as
/// a plain positional (not a clap `value_parser` list) so it does not collide with the release
/// `--channel` single-source check, which scans `lib.rs` for the first `value_parser = [..]`.
pub fn json_stdin_requested(positional: Option<&str>) -> Result<bool> {
    match positional {
        None => Ok(false),
        Some(v) if v == STDIN_SENTINEL => Ok(true),
        Some(other) => Err(NxfError::validation(format!(
            "unexpected argument '{other}'; pass '-' to read the write payload from STDIN as JSON"
        ))),
    }
}

/// A generous upper bound on a single long-text field read from STDIN or a file. These fields are
/// human/agent prose, not blobs — 64 MiB is orders of magnitude above any real
/// description/design/DoD while turning an otherwise unbounded read (a piped `/dev/zero`, an
/// accidental huge file) into a clean `validation` error instead of allocating until OOM. This is
/// a defensive guard, NOT a security boundary: a local CLI runs with the user's own privileges.
const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;

/// Read `reader` to a UTF-8 `String`, refusing input larger than `limit` bytes. `source` names the
/// origin for error messages (`"STDIN"`, `"file '…'"`). Over the limit is a `validation` error
/// naming it; an I/O failure is `io`; non-UTF-8 content is `validation`. `limit` is a parameter
/// (not the const inlined) so the cap is unit-testable without a 64 MiB fixture.
fn read_capped(reader: impl Read, limit: u64, source: &str) -> Result<String> {
    // Read one byte past the limit: if we manage to, the input is over-cap.
    let mut buf = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|e| NxfError::io(format!("reading {source}: {e}")))?;
    if buf.len() as u64 > limit {
        return Err(NxfError::validation(format!(
            "{source} exceeds the {} MiB long-text input limit",
            limit / (1024 * 1024)
        )));
    }
    String::from_utf8(buf)
        .map_err(|_| NxfError::validation(format!("{source} is not valid UTF-8 text")))
}

/// Read a file as UTF-8 text, verbatim (capped at [`MAX_INPUT_BYTES`]). A read failure (missing /
/// unreadable) is an `io` error; non-UTF-8 content is a `validation` error — both before any
/// write, so a bad path changes nothing. Errors name the path so an agent can self-correct.
fn read_file(path: &str) -> Result<String> {
    let file = std::fs::File::open(path)
        .map_err(|e| NxfError::io(format!("reading file '{path}': {e}")))?;
    read_capped(file, MAX_INPUT_BYTES, &format!("file '{path}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    #[test]
    fn read_capped_accepts_input_at_or_under_the_limit() {
        assert_eq!(read_capped(b"hello".as_slice(), 5, "x").unwrap(), "hello");
        assert_eq!(read_capped(b"hi".as_slice(), 64, "x").unwrap(), "hi");
        assert_eq!(read_capped(b"".as_slice(), 0, "x").unwrap(), "");
    }

    #[test]
    fn read_capped_rejects_input_over_the_limit_with_a_validation_error() {
        let err = read_capped(b"too long".as_slice(), 3, "STDIN").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(
            err.msg.contains("STDIN") && err.msg.contains("limit"),
            "{}",
            err.msg
        );
    }

    #[test]
    fn read_capped_rejects_non_utf8_with_a_validation_error() {
        let err = read_capped([0xff, 0xfe].as_slice(), 64, "file 'x'").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.msg.contains("UTF-8"), "{}", err.msg);
    }
}
