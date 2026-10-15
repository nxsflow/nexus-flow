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

/// Validate a `datetime` custom-field value (nxf 6j6v.vbp6) and return its stored form. The input
/// must be an RFC 3339 timestamp WITH a time zone (`Z` or `±hh:mm`); a plain date or a local time
/// without an offset is refused. The stored form is the same instant in UTC at whole seconds,
/// `YYYY-MM-DDTHH:MM:SSZ`: fractional seconds are dropped and the given offset is applied, then
/// discarded. One fixed shape means the text order of stored values IS their chronological order,
/// which is what every comparison downstream relies on (the SQL sort, the hosted fold, the ranking
/// comparator's text arm). A UTC year outside `0000..=9999` is refused for the same reason.
/// Surrounding whitespace is trimmed first, so a `--set` value and a list entry (which the write
/// seam trims anyway) accept the same spellings (review of PR #63, Code Quality #6).
pub fn datetime(value: &str) -> Result<String> {
    let value = value.trim();
    let refuse = || {
        NxfError::validation(format!(
            "'{value}' is not an RFC 3339 date-time with a time zone \
             (e.g. 2026-10-12T08:30:00Z or 2026-10-12T10:30:00+02:00)"
        ))
    };
    let utc = OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|_| refuse())?
        .checked_to_offset(time::UtcOffset::UTC)
        .filter(|utc| (0..=9999).contains(&utc.year()))
        .ok_or_else(refuse)?;
    Ok(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        utc.year(),
        u8::from(utc.month()),
        utc.day(),
        utc.hour(),
        utc.minute(),
        utc.second()
    ))
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

/// Validate the ids a supersede replaces (6j6v.c0kn): at least one and at most
/// [`MAX_SUPERSEDED`](nexus_flow_core::model::MAX_SUPERSEDED), each non-empty, at most
/// [`MAX_CHUNK_ID_BYTES`](nexus_flow_core::model::MAX_CHUNK_ID_BYTES) and free of the two separators
/// a supersede value uses. Whether an id names a chunk this replica holds is not asked: a chunk
/// may still be on its way, and a supersede that arrives first hides it anyway.
pub fn chunk_ids<'a>(ids: &[&'a str]) -> Result<Vec<&'a str>> {
    use nexus_flow_core::model::{MAX_CHUNK_ID_BYTES, MAX_SUPERSEDED};
    if ids.is_empty() {
        return Err(NxfError::validation(
            "a supersede must name at least one chunk it replaces",
        ));
    }
    if ids.len() > MAX_SUPERSEDED {
        return Err(NxfError::validation(format!(
            "a supersede replaces at most {MAX_SUPERSEDED} chunks, this one {}: supersede in rounds",
            ids.len()
        )));
    }
    if let Some(bad) = ids
        .iter()
        .find(|id| !nexus_flow_core::model::is_valid_chunk_id(id))
    {
        return Err(NxfError::validation(format!(
            "'{}' is not a chunk id: it must be 1 to {MAX_CHUNK_ID_BYTES} bytes and free of \
             U+001E and U+001F",
            bad.escape_debug()
        )));
    }
    Ok(ids.to_vec())
}

/// Refuse a chunk on an item whose id is longer than a chunk op may name
/// ([`MAX_CHUNK_ID_BYTES`](nexus_flow_core::model::MAX_CHUNK_ID_BYTES)): the reducer would never
/// fold it.
pub fn chunk_item(id: &str) -> Result<&str> {
    let limit = nexus_flow_core::model::MAX_CHUNK_ID_BYTES;
    if id.len() > limit {
        return Err(NxfError::validation(format!(
            "an item id carries chunks only up to {limit} bytes"
        )));
    }
    Ok(id)
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

    #[test]
    fn datetime_normalizes_any_offset_to_utc_seconds() {
        // The offset is applied, not dropped: 10:30 at +02:00 is 08:30 UTC.
        assert_eq!(
            datetime("2026-10-12T10:30:00+02:00").unwrap(),
            "2026-10-12T08:30:00Z"
        );
        assert_eq!(
            datetime("2026-10-12T08:30:00Z").unwrap(),
            "2026-10-12T08:30:00Z"
        );
        // A negative offset can cross midnight into the next day.
        assert_eq!(
            datetime("2026-12-31T23:00:00-05:00").unwrap(),
            "2027-01-01T04:00:00Z"
        );
        // Lowercase separators are RFC 3339 too.
        assert_eq!(
            datetime("2026-10-12t08:30:00z").unwrap(),
            "2026-10-12T08:30:00Z"
        );
    }

    #[test]
    fn datetime_drops_fractional_seconds() {
        // Fixed precision keeps text order chronological: `…00.5Z` would sort BEFORE `…00Z`.
        assert_eq!(
            datetime("2026-10-12T08:30:00.000Z").unwrap(),
            "2026-10-12T08:30:00Z"
        );
        assert_eq!(
            datetime("2026-10-12T08:30:59.999999+00:00").unwrap(),
            "2026-10-12T08:30:59Z"
        );
    }

    #[test]
    fn datetime_refuses_a_value_without_a_time_zone() {
        for v in [
            "2026-10-12",
            "2026-10-12T08:30:00",
            "2026-10-12T08:30",
            "2026-10-12 08:30:00",
            "not-a-date",
            "2026-13-01T00:00:00Z",
            "",
        ] {
            let err = datetime(v).expect_err(v);
            assert_eq!(err.kind, crate::error::ErrorKind::Validation, "{v}");
            assert!(err.msg.contains("RFC 3339"), "{v}: {}", err.msg);
        }
    }

    #[test]
    fn datetime_trims_surrounding_whitespace() {
        assert_eq!(
            datetime("  2026-10-12T10:30:00+02:00\n").unwrap(),
            "2026-10-12T08:30:00Z"
        );
        assert!(datetime("   ").is_err());
    }

    #[test]
    fn datetime_stores_a_leap_second_as_the_second_before() {
        // RFC 3339 allows `:60`; the `time` crate reads it as the last representable instant of
        // that minute, so the stored form keeps the fixed shape and its chronological text order.
        assert_eq!(
            datetime("2016-12-31T23:59:60Z").unwrap(),
            "2016-12-31T23:59:59Z"
        );
        assert_eq!(
            datetime("2017-01-01T00:59:60+01:00").unwrap(),
            "2016-12-31T23:59:59Z"
        );
    }

    #[test]
    fn datetime_accepts_a_space_for_the_t_with_a_time_zone_and_stores_the_t() {
        // RFC 3339 §5.6 NOTE permits a space for the `T`, and the `time` crate reads it. With a
        // zone it is the same instant and is stored in the one shape; without a zone it is still
        // refused (`datetime_refuses_a_value_without_a_time_zone`).
        assert_eq!(
            datetime("2026-10-12 08:30:00Z").unwrap(),
            "2026-10-12T08:30:00Z"
        );
        assert_eq!(
            datetime("2026-10-12 10:30:00+02:00").unwrap(),
            "2026-10-12T08:30:00Z"
        );
    }

    #[test]
    fn datetime_refuses_a_utc_year_outside_four_digits() {
        // In range as given, out of range once moved to UTC: refused, not stored with a 5-digit or
        // negative year that would break the text order.
        assert!(datetime("9999-12-31T23:30:00-01:00").is_err());
        assert!(datetime("0000-01-01T00:30:00+01:00").is_err());
        assert_eq!(
            datetime("0000-01-01T00:30:00Z").unwrap(),
            "0000-01-01T00:30:00Z"
        );
    }
}
