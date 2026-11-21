//! Timestamp handling for displaying post and comment ages.
//!
//! Deliberately dependency-free. Lemmy returns UTC timestamps in a fixed shape
//! (`YYYY-MM-DDTHH:MM:SS[.ffffff]Z`), so parsing is a positional field split
//! rather than a general date grammar.
//!
//! The correctness risk in hand-written calendar code is real, so the epoch
//! conversion is a pure function tested against epoch seconds computed
//! independently (with Python's `datetime`) — including a leap day and a
//! year boundary. The age formatter is likewise a pure function of a duration,
//! tested at each threshold.

/// Seconds since the Unix epoch for an RFC 3339 UTC timestamp.
///
/// Returns `None` when the value is not in the expected shape or any component
/// is out of range, so a surprise from the server degrades to "no age shown"
/// rather than a wrong one.
pub fn parse_rfc3339_utc_secs(value: &str) -> Option<i64> {
    let text = value.trim();
    // The field split below indexes by byte, which is only sound on ASCII: a
    // multi-byte character straddling one of those boundaries (a timestamp from
    // a federated instance is text from a stranger) makes `&text[17..19]` panic,
    // and this runs while drawing every feed row, so the panic would take the
    // window down. Every timestamp RFC 3339 describes here is ASCII, so anything
    // else is simply not one we can read. Rejecting it is what the rest of this
    // function promises; panicking is not.
    if !text.is_ascii() {
        return None;
    }
    let bytes = text.as_bytes();
    // "YYYY-MM-DDTHH:MM:SSZ" is the minimum length.
    if bytes.len() < 20 {
        return None;
    }

    // Separators exactly where RFC 3339 puts them.
    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    if !matches!(bytes[10], b'T' | b't' | b' ') {
        return None;
    }

    let year = digits(&text[0..4])?;
    let month = digits(&text[5..7])?;
    let day = digits(&text[8..10])?;
    let hour = digits(&text[11..13])?;
    let minute = digits(&text[14..16])?;
    let second = digits(&text[17..19])?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    // Everything after the seconds: optional sub-second digits, then a UTC
    // designator.
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits_end = fraction
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(fraction.len());
        if digits_end == 0 {
            return None; // "." with no digits
        }
        rest = &fraction[digits_end..];
    }
    match rest {
        "Z" | "z" | "+00:00" | "-00:00" => {}
        // Any other offset would need zone arithmetic; refuse rather than
        // silently treat a non-UTC value as UTC.
        _ => return None,
    }

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn digits(slice: &str) -> Option<i64> {
    if slice.is_empty() || !slice.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    slice.parse::<i64>().ok()
}

/// Days from 1970-01-01 to the given civil date.
///
/// Howard Hinnant's `days_from_civil`, which is exact for all dates in range
/// and needs no lookup tables.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400; // [0, 399]
    let month_prime = (month + 9) % 12; // [0, 11], March = 0
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1; // [0, 365]
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Age in seconds formatted compactly: `just now`, `5m ago`, `3h ago`,
/// `2d ago`, `8mo ago`, `4y ago`.
pub fn humanise_age(seconds: i64) -> String {
    // A small negative delta means clock skew between us and the instance;
    // "just now" is truer than "in 3 seconds".
    if seconds < 45 {
        return "just now".to_string();
    }
    if seconds < 90 {
        return "1m ago".to_string();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = seconds / 3_600;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = seconds / 86_400;
    if days < 30 {
        return format!("{days}d ago");
    }
    let months = days / 30;
    if months < 12 {
        return format!("{months}mo ago");
    }
    format!("{}y ago", days / 365)
}

/// Age of an RFC 3339 timestamp relative to `now_secs`.
pub fn age_of(published: &str, now_secs: i64) -> Option<String> {
    parse_rfc3339_utc_secs(published).map(|then| humanise_age(now_secs - then))
}

/// Current Unix time, or 0 if the system clock is before the epoch.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values computed independently with Python:
    /// `int(datetime.fromisoformat(s.replace("Z","+00:00")).timestamp())`.
    #[test]
    fn parses_timestamps_to_the_expected_epoch_seconds() {
        let cases = [
            ("1970-01-01T00:00:00Z", 0),
            ("2026-10-14T15:44:02Z", 1_791_992_642),
            ("2024-02-29T12:00:00Z", 1_709_208_000),
            ("2000-03-01T00:00:00.123456Z", 951_868_800),
            ("2026-12-31T23:59:59Z", 1_798_761_599),
            ("2027-01-01T00:00:00Z", 1_798_761_600),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse_rfc3339_utc_secs(input),
                Some(expected),
                "wrong epoch for {input}"
            );
        }
    }

    #[test]
    fn consecutive_days_differ_by_exactly_one_day() {
        let first = parse_rfc3339_utc_secs("2026-02-28T00:00:00Z").expect("parse");
        let second = parse_rfc3339_utc_secs("2026-03-01T00:00:00Z").expect("parse");
        // 2026 is not a leap year, so this crosses a plain month boundary.
        assert_eq!(second - first, 86_400);
    }

    #[test]
    fn leap_year_day_arithmetic_holds() {
        let feb28 = parse_rfc3339_utc_secs("2024-02-28T00:00:00Z").expect("parse");
        let feb29 = parse_rfc3339_utc_secs("2024-02-29T00:00:00Z").expect("parse");
        let mar1 = parse_rfc3339_utc_secs("2024-03-01T00:00:00Z").expect("parse");
        assert_eq!(feb29 - feb28, 86_400);
        assert_eq!(mar1 - feb29, 86_400);
    }

    #[test]
    fn accepts_a_utc_offset_and_lowercase_designator() {
        assert_eq!(
            parse_rfc3339_utc_secs("2026-10-14T15:44:02+00:00"),
            Some(1_791_992_642)
        );
        assert_eq!(
            parse_rfc3339_utc_secs("2026-10-14t15:44:02z"),
            Some(1_791_992_642)
        );
    }

    #[test]
    fn rejects_values_it_cannot_trust() {
        for bad in [
            "",
            "not a date",
            "2026-10-14",
            "2026-10-14T15:44",
            "2026-13-01T00:00:00Z",  // month out of range
            "2026-10-14T25:00:00Z",  // hour out of range
            "2026-10-14T15:44:02+02:00", // non-UTC offset
            "2026-10-14T15:44:02.",
            "20xx-10-14T15:44:02Z",
        ] {
            assert_eq!(
                parse_rfc3339_utc_secs(bad),
                None,
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn formats_ages_at_each_threshold() {
        assert_eq!(humanise_age(-5), "just now", "clock skew reads as now");
        assert_eq!(humanise_age(0), "just now");
        assert_eq!(humanise_age(44), "just now");
        assert_eq!(humanise_age(60), "1m ago");
        assert_eq!(humanise_age(59 * 60), "59m ago");
        assert_eq!(humanise_age(3_600), "1h ago");
        assert_eq!(humanise_age(23 * 3_600), "23h ago");
        assert_eq!(humanise_age(86_400), "1d ago");
        assert_eq!(humanise_age(29 * 86_400), "29d ago");
        assert_eq!(humanise_age(60 * 86_400), "2mo ago");
        assert_eq!(humanise_age(400 * 86_400), "1y ago");
    }

    /// A timestamp arrives from an instance, and an instance can send anything.
    /// A multi-byte character across a field boundary used to abort the parse
    /// with a slicing panic — which is a crash of the whole app, mid-frame, not
    /// an error state. Whatever the text is, the answer must be `None`.
    #[test]
    fn never_panics_on_hostile_input() {
        let valid = "2026-10-14T15:44:02Z";
        let mut inputs: Vec<String> = Vec::new();
        for (index, _) in valid.char_indices() {
            for odd in ['é', '😀', '日'] {
                let mut text = String::new();
                text.push_str(&valid[..index]);
                text.push(odd);
                text.push_str(&valid[index..]);
                inputs.push(text);
            }
        }
        for odd in ['é', '😀', '日'] {
            inputs.push(odd.to_string());
            inputs.push(format!("{odd}{valid}"));
            inputs.push(format!("{valid}{odd}"));
        }

        for input in inputs {
            assert_eq!(
                parse_rfc3339_utc_secs(&input),
                None,
                "{input:?} is not a UTC timestamp: it must be rejected, not panic"
            );
        }
    }

    #[test]
    fn age_of_reports_none_for_an_unparseable_timestamp() {
        assert_eq!(age_of("garbage", 0), None);
        // One hour after the timestamp.
        assert_eq!(
            age_of("2026-10-14T15:44:02Z", 1_791_992_642 + 3_600).as_deref(),
            Some("1h ago")
        );
    }
}
