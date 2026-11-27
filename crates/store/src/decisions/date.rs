//! A decision's date (RFC 0003, "Writing a decision"): a real calendar date, `YYYY-MM-DD`, today's
//! in UTC unless the writer gives the day the decision was made.

use std::time::{SystemTime, UNIX_EPOCH};

/// Whether `text` is a calendar date written `YYYY-MM-DD`.
pub fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || !digits(0..4) {
        return false;
    }
    if !digits(5..7) || !digits(8..10) {
        return false;
    }
    let number = |r: std::ops::Range<usize>| text[r].parse::<u32>().unwrap_or(0);
    let (year, month, day) = (number(0..4), number(5..7), number(8..10));
    (1..=days_in(year, month)).contains(&day)
}

/// Today's date in UTC, `YYYY-MM-DD`.
pub fn today() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (year, month, day) = civil(i64::try_from(seconds / 86_400).unwrap_or(0));
    format!("{year:04}-{month:02}-{day:02}")
}

fn days_in(year: u32, month: u32) -> u32 {
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    }
}

/// The calendar date `days` days after 1970-01-01, by Howard Hinnant's `civil_from_days`.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_since_the_epoch_are_a_calendar_date() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_784), (2026, 11, 27));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn a_date_is_real_and_written_in_full() {
        assert!(is_date("2024-02-29"));
        assert!(!is_date("2026-02-29"));
        assert!(!is_date("2026-13-01"));
        assert!(!is_date("2026-1-01"));
        assert!(!is_date("2026-11-00"));
    }
}
