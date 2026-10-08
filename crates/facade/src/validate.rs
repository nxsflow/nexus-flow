//! Shared input validation for the write paths. Loud rejection at write time beats a
//! silently-stored bad value the derivation later has to cope with. Lives in the facade (not
//! the CLI) so every write seam — the CLI, the MCP server, an embedding app — validates
//! identically through the one shared write layer (E5 #9t7.5).

use crate::error::{NxfError, Result};
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{Date, OffsetDateTime};

/// Validate the `actor` a caller supplies to a write, returning the stored (trimmed) form
/// (review finding Code Quality #1, PR #311). [`label`]'s rules, for [`label`]'s reason: a blank
/// actor is a caller mistake, and the value goes straight onto `op.author` of an append-only log
/// where attribution cannot be repaired afterwards.
///
/// The rule itself lives once, at the substrate ([`nxs_foundation::model::validate_author`]) — this
/// is flow's name for it, so a write path reads `validate::actor(actor)?` beside its other checks.
/// It is the `Result` half of the guarantee `Store::emit` asserts: the assert is the last-resort
/// backstop against an unattributed op, this is the input check that means an embedding app gets a
/// rejection instead of a panic.
pub fn actor(value: &str) -> Result<&str> {
    nxs_foundation::model::validate_author(value)
}

/// Accept either a plain calendar date (`YYYY-MM-DD`) or a full RFC3339 timestamp,
/// returning the value unchanged. Anything else is a validation error.
pub fn iso_date(value: &str) -> Result<String> {
    let date = format_description!("[year]-[month]-[day]");
    if Date::parse(value, &date).is_ok() || OffsetDateTime::parse(value, &Rfc3339).is_ok() {
        Ok(value.to_string())
    } else {
        Err(NxfError::validation(format!(
            "'{value}' is not an ISO-8601 date (YYYY-MM-DD) or RFC3339 timestamp"
        )))
    }
}

/// Canonicalize + validate a user label (h89s): trim surrounding whitespace, reject an empty
/// result, and reject the U+001F composite separator (it would break the OR-set's `item␟label`
/// target_id). Returns the stored form. The ONE definition every label path shares — the facade
/// write seam (`label_add`/`remove`) surfaces the `Err`, the beads migration skips + warns on it —
/// so a label is trimmed/validated identically no matter which seam attaches it (E5 #9t7.5).
pub fn label(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(NxfError::validation("a label must not be blank"));
    }
    if trimmed.contains('\u{1f}') {
        return Err(NxfError::validation(
            "a label must not contain the U+001F control character",
        ));
    }
    Ok(trimmed.to_string())
}

/// Normalize a chat thread id for a thread link (nxf 6j6v.8dbe): trimmed, non-blank,
/// separator-free — [`label`]'s rules, for [`label`]'s reasons (a blank id is a caller mistake, and
/// a U+001F would make the reducer's composite parse fail so the link would silently never fold).
///
/// What it deliberately does NOT do is check the id's SHAPE or whether it resolves. A thread lives
/// in the chat store's id space, which flow neither owns nor can read; the engine's job here is to
/// hold the address, not to adjudicate it.
pub fn thread_id(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(NxfError::validation("a thread id must not be blank"));
    }
    if trimmed.contains('\u{1f}') {
        return Err(NxfError::validation(
            "a thread id must not contain the U+001F control character",
        ));
    }
    Ok(trimmed.to_string())
}

/// Validate an item's chunk field name (6j6v.c0kn) — see
/// [`is_valid_chunk_field`](nexus_flow_core::model::is_valid_chunk_field) for the rule.
pub fn chunk_field(field: &str) -> Result<&str> {
    if nexus_flow_core::model::is_valid_chunk_field(field) {
        Ok(field)
    } else {
        Err(NxfError::validation(format!(
            "'{field}' is not a chunk field: 1 to 64 ASCII letters, digits, '_', '-' or '.'"
        )))
    }
}

/// Validate the ids a supersede replaces (6j6v.c0kn): at least one, each non-empty and free of the
/// two separators a supersede value uses. Whether an id names a chunk this replica holds is not
/// asked: a chunk may still be on its way, and a supersede that arrives first hides it anyway.
pub fn chunk_ids<'a>(ids: &[&'a str]) -> Result<Vec<&'a str>> {
    if ids.is_empty() {
        return Err(NxfError::validation(
            "a supersede must name at least one chunk it replaces",
        ));
    }
    if let Some(bad) = ids
        .iter()
        .find(|id| !nexus_flow_core::model::is_valid_chunk_id(id))
    {
        return Err(NxfError::validation(format!(
            "'{}' is not a chunk id: it must be non-empty and free of U+001E and U+001F",
            bad.escape_debug()
        )));
    }
    Ok(ids.to_vec())
}

/// Refuse a chunk op whose `value` would pass [`MAX_CHUNK_BYTES`](nexus_flow_core::model::MAX_CHUNK_BYTES)
/// (6j6v.c0kn) — never cut it short: a truncated opaque chunk is a corrupt one. `bytes` is the
/// whole value, the replaced ids included.
pub fn chunk_size(bytes: usize) -> Result<()> {
    let limit = nexus_flow_core::model::MAX_CHUNK_BYTES;
    if bytes > limit {
        return Err(NxfError::validation(format!(
            "a chunk op carries at most {limit} bytes, this one {bytes}: split the chunk, or \
             supersede fewer chunks at once"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_id_trims_and_rejects_blank_or_separator() {
        assert_eq!(thread_id("  m-1  ").unwrap(), "m-1");
        assert!(thread_id("   ").is_err());
        assert!(thread_id("m-\u{1f}1").is_err());
    }

    #[test]
    fn accepts_plain_date_and_rfc3339() {
        assert!(iso_date("2026-06-09").is_ok());
        assert!(iso_date("2026-06-09T12:30:00Z").is_ok());
    }

    #[test]
    fn rejects_garbage_and_impossible_dates() {
        assert!(iso_date("not-a-date").is_err());
        assert!(iso_date("2026-13-01").is_err());
        assert!(iso_date("2026/06/09").is_err());
    }
}
