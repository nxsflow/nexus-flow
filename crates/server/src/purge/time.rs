//! RFC 3339 instants, parsed and written without a date crate: the purge reads `wall_clock` (the
//! author device's clock, client-supplied text) and reports the latest one as a UTC instant, and it
//! stamps its own `started_at`/`finished_at`. Both directions are the proleptic Gregorian calendar
//! in whole seconds plus nanoseconds since the Unix epoch.

/// An instant: seconds since 1970-01-01T00:00:00Z and the nanoseconds within that second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant {
    pub secs: i64,
    pub nanos: u32,
}

/// Parse `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)` (RFC 3339 §5.6, `T`/`t` and `Z`/`z` either
/// case). Anything else — an empty string, a date without a time or an offset, an impossible
/// day — is `None`: the caller counts it as unparseable rather than guessing.
pub fn parse_rfc3339(text: &str) -> Option<Instant> {
    let b = text.as_bytes();
    // date-time = full-date "T" partial-time time-offset; the fixed part is 19 bytes.
    if b.len() < 20 {
        return None;
    }
    let num = |range: std::ops::Range<usize>| -> Option<u32> {
        let digits = b.get(range)?;
        if !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        digits
            .iter()
            .try_fold(0u32, |acc, d| Some(acc * 10 + u32::from(d - b'0')))
    };
    let sep = |i: usize, c: &[u8]| b.get(i).is_some_and(|x| c.contains(x));
    if !(sep(4, b"-") && sep(7, b"-") && sep(10, b"Tt") && sep(13, b":") && sep(16, b":")) {
        return None;
    }
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year as i64, month)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut i = 19;
    let mut nanos = 0u32;
    if b[i] == b'.' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let digits = &b[start..i];
        if digits.is_empty() {
            return None;
        }
        // Nanoseconds: the first nine digits, padded; finer digits cannot be represented.
        for k in 0..9 {
            nanos = nanos * 10 + digits.get(k).map_or(0, |d| u32::from(d - b'0'));
        }
    }
    let offset_secs: i64 = match &b[i..] {
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] => {
            let field = |a: u8, c: u8| -> Option<i64> {
                (a.is_ascii_digit() && c.is_ascii_digit())
                    .then(|| i64::from(a - b'0') * 10 + i64::from(c - b'0'))
            };
            let (h, m) = (field(*h1, *h2)?, field(*m1, *m2)?);
            if h > 23 || m > 59 {
                return None;
            }
            let secs = h * 3600 + m * 60;
            if *sign == b'+' {
                secs
            } else {
                -secs
            }
        }
        _ => return None,
    };
    let days = days_from_civil(year as i64, month, day);
    let local = days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second);
    Some(Instant {
        secs: local - offset_secs,
        nanos,
    })
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(year) => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day number since 1970-01-01 (H. Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Write an instant as `YYYY-MM-DDTHH:MM:SS[.fraction]Z`, the fraction without trailing zeros and
/// left out when it is zero.
pub fn format_utc(instant: Instant) -> String {
    let days = instant.secs.div_euclid(86_400);
    let rem = instant.secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (rem / 3600, rem % 3600 / 60, rem % 60);
    let mut text = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}");
    if instant.nanos > 0 {
        let fraction = format!("{:09}", instant.nanos);
        text.push('.');
        text.push_str(fraction.trim_end_matches('0'));
    }
    text.push('Z');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(text: &str) -> String {
        format_utc(parse_rfc3339(text).unwrap_or_else(|| panic!("{text:?} parses")))
    }

    #[test]
    fn a_utc_instant_round_trips() {
        assert_eq!(utc("2026-10-18T21:04:11Z"), "2026-10-18T21:04:11Z");
        assert_eq!(utc("1970-01-01T00:00:00Z"), "1970-01-01T00:00:00Z");
        assert_eq!(
            parse_rfc3339("1970-01-01T00:00:01Z"),
            Some(Instant { secs: 1, nanos: 0 })
        );
    }

    #[test]
    fn an_offset_is_normalised_to_utc() {
        assert_eq!(utc("2026-10-19T01:30:00+02:00"), "2026-10-18T23:30:00Z");
        assert_eq!(utc("2026-12-31T23:30:00-01:00"), "2027-01-01T00:30:00Z");
        assert_eq!(utc("2026-10-18t21:04:11z"), "2026-10-18T21:04:11Z");
    }

    #[test]
    fn a_fraction_is_kept_without_trailing_zeros() {
        assert_eq!(utc("2026-10-18T21:04:11.250Z"), "2026-10-18T21:04:11.25Z");
        assert_eq!(utc("2026-10-18T21:04:11.000Z"), "2026-10-18T21:04:11Z");
        assert_eq!(
            utc("2026-10-18T21:04:11.123456789Z"),
            "2026-10-18T21:04:11.123456789Z"
        );
    }

    #[test]
    fn leap_days_and_month_ends_are_checked() {
        assert!(parse_rfc3339("2024-02-29T00:00:00Z").is_some());
        assert!(parse_rfc3339("2026-02-29T00:00:00Z").is_none());
        assert!(parse_rfc3339("2026-04-31T00:00:00Z").is_none());
        assert!(parse_rfc3339("2026-13-01T00:00:00Z").is_none());
        assert_eq!(utc("2000-02-29T12:00:00Z"), "2000-02-29T12:00:00Z");
    }

    #[test]
    fn what_is_not_an_rfc3339_instant_is_refused() {
        for text in [
            "",
            "2026-10-18",
            "2026-10-18T21:04:11",
            "2026-10-18 21:04:11Z",
            "2026-10-18T21:04Z",
            "2026-10-18T24:00:00Z",
            "2026-10-18T21:60:00Z",
            "2026-10-18T21:04:11.Z",
            "2026-10-18T21:04:11+0200",
            "26-10-18T21:04:11Z",
            "yesterday",
            "2026-10-18T21:04:11Z ",
            "２０２６-10-18T21:04:11Z",
        ] {
            assert_eq!(parse_rfc3339(text), None, "{text:?}");
        }
    }

    #[test]
    fn later_instants_compare_greater_whatever_their_offset() {
        let a = parse_rfc3339("2026-10-18T23:00:00+02:00").unwrap();
        let b = parse_rfc3339("2026-10-18T22:00:00Z").unwrap();
        assert!(b > a, "22:00Z is after 21:00Z");
    }
}
