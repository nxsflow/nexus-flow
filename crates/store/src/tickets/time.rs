//! A ticket's times (RFC 0002, "The ticket"): `createdAt` and `updatedAt` are RFC 3339 instants
//! in UTC, to the second, which `nxs` writes; `due` and `deferUntil` are RFC 3339 times the
//! writer gives, a full date or a date and time with its offset, stored as written.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::decisions::date::{civil, is_date};

/// Now, as an RFC 3339 instant in UTC to the second, such as `2026-11-28T09:00:00Z`.
pub fn now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (year, month, day) = civil(i64::try_from(seconds / 86_400).unwrap_or(0));
    let second = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        second / 3600,
        second / 60 % 60,
        second % 60
    )
}

/// Whether `text` is an RFC 3339 `full-date`, such as `2026-12-01`, or `date-time`, such as
/// `2026-12-01T09:00:00Z` or `2026-12-01T09:00:00.5+01:00`.
pub fn is_time(text: &str) -> bool {
    if text.len() <= 10 {
        return is_date(text);
    }
    let (date, rest) = text.split_at(10);
    let Some(rest) = rest.strip_prefix(['T', 't']) else {
        return false;
    };
    is_date(date) && is_time_of_day(rest)
}

/// `HH:MM:SS`, an optional fraction, and `Z` or `±HH:MM`.
fn is_time_of_day(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() < 9 || !clock(&b[..8], 23, 60) {
        return false;
    }
    let mut rest = &b[8..];
    if let Some(fraction) = rest.strip_prefix(b".") {
        let digits = fraction.iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return false;
        }
        rest = &fraction[digits..];
    }
    match rest {
        [b'Z' | b'z'] => true,
        [b'+' | b'-', offset @ ..] => offset.len() == 5 && clock(offset, 23, 59),
        _ => false,
    }
}

/// `HH:MM` or `HH:MM:SS`, with hours up to `hours` and the last part up to `last`; a leap second
/// is `60`.
fn clock(b: &[u8], hours: u8, last: u8) -> bool {
    let two = |i: usize| -> Option<u8> {
        let pair = b.get(i..i + 2)?;
        pair.iter()
            .all(u8::is_ascii_digit)
            .then(|| (pair[0] - b'0') * 10 + (pair[1] - b'0'))
    };
    let ok = |i, max| two(i).is_some_and(|n| n <= max);
    match b.len() {
        5 => b[2] == b':' && ok(0, hours) && ok(3, last),
        8 => b[2] == b':' && b[5] == b':' && ok(0, hours) && ok(3, 59) && ok(6, last),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_3339_dates_and_times_are_times() {
        for text in [
            "2026-12-01",
            "2026-12-01T09:00:00Z",
            "2026-12-01t09:00:00z",
            "2026-12-01T09:00:00.25+01:00",
            "2026-12-31T23:59:60-05:30",
        ] {
            assert!(is_time(text), "{text}");
        }
        for text in [
            "",
            "2026-02-30",
            "2026-12-01 09:00:00Z",
            "2026-12-01T09:00:00",
            "2026-12-01T24:00:00Z",
            "2026-12-01T09:00Z",
            "2026-12-01T09:00:00.Z",
            "2026-12-01T09:00:00+1:00",
            "tomorrow",
        ] {
            assert!(!is_time(text), "{text}");
        }
    }

    #[test]
    fn now_is_an_instant_to_the_second() {
        let now = now();
        assert_eq!(now.len(), 20, "{now}");
        assert!(is_time(&now), "{now}");
    }
}
