//! Timestamp helpers. Timestamps in the schema are writer-set RFC 3339
//! strings (design decision D10): trivially JSON-exportable and independent
//! of automerge's history.

use chrono::{SecondsFormat, Utc};

/// Current time as an RFC 3339 UTC string with microsecond precision,
/// e.g. `2026-06-03T14:37:32.946678Z` (matches the beads JSONL style).
pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// Parse a user-supplied wake time (`braid defer --until`) into the
/// canonical RFC 3339 UTC form [`now_rfc3339`] emits.
///
/// Accepted forms:
/// - full RFC 3339 (`2026-07-01T09:00:00Z`; offsets are normalized to UTC)
/// - bare date (`2026-07-01` → that day at 00:00:00 UTC)
/// - duration relative to `now`: `<N>h`, `<N>d`, `<N>w`
///
/// `None` when the input matches no form (callers own the error message).
pub fn parse_until(input: &str, now: &str) -> Option<String> {
    use chrono::{DateTime, Duration, NaiveDate};

    let canonical = |t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Micros, true);

    if let Ok(t) = DateTime::parse_from_rfc3339(input) {
        return Some(canonical(t.with_timezone(&Utc)));
    }
    if let Ok(d) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        return Some(canonical(d.and_hms_opt(0, 0, 0)?.and_utc()));
    }

    let unit = input.chars().last()?;
    let n: i64 = input[..input.len() - unit.len_utf8()].parse().ok()?;
    if n < 0 {
        return None;
    }
    let dur = match unit {
        'h' => Duration::hours(n),
        'd' => Duration::days(n),
        'w' => Duration::weeks(n),
        _ => return None,
    };
    let now = DateTime::parse_from_rfc3339(now).ok()?;
    Some(canonical(now.with_timezone(&Utc) + dur))
}

/// Whether timestamp `a` is strictly after `b`, comparing parsed instants
/// (lexicographic comparison is wrong across differing precisions, e.g.
/// `...:00Z` vs `...:00.5Z`). `None` when either fails to parse — callers
/// decide how to be conservative.
pub fn is_after(a: &str, b: &str) -> Option<bool> {
    let a = chrono::DateTime::parse_from_rfc3339(a).ok()?;
    let b = chrono::DateTime::parse_from_rfc3339(b).ok()?;
    Some(a > b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-06-04T12:00:00.000000Z";

    #[test]
    fn parse_until_accepts_rfc3339_and_normalizes_to_utc() {
        assert_eq!(
            parse_until("2026-07-01T09:00:00Z", NOW).as_deref(),
            Some("2026-07-01T09:00:00.000000Z")
        );
        // an offset timestamp lands on the same instant in UTC
        assert_eq!(
            parse_until("2026-07-01T09:00:00+02:00", NOW).as_deref(),
            Some("2026-07-01T07:00:00.000000Z")
        );
    }

    #[test]
    fn parse_until_accepts_bare_date_as_midnight_utc() {
        assert_eq!(parse_until("2026-07-01", NOW).as_deref(), Some("2026-07-01T00:00:00.000000Z"));
    }

    #[test]
    fn parse_until_accepts_durations_relative_to_now() {
        assert_eq!(parse_until("36h", NOW).as_deref(), Some("2026-06-06T00:00:00.000000Z"));
        assert_eq!(parse_until("7d", NOW).as_deref(), Some("2026-06-11T12:00:00.000000Z"));
        assert_eq!(parse_until("2w", NOW).as_deref(), Some("2026-06-18T12:00:00.000000Z"));
        assert_eq!(parse_until("0d", NOW).as_deref(), Some(NOW), "0d = now, immediate wake");
    }

    #[test]
    fn parse_until_rejects_what_it_should() {
        for bad in ["", "soon", "3x", "-3d", "d", "2026-13-40", "7 d", "3.5d", "7µ"] {
            assert_eq!(parse_until(bad, NOW), None, "should reject {bad:?}");
        }
        // a duration against an unparseable `now` cannot resolve
        assert_eq!(parse_until("7d", "garbage"), None);
    }

    #[test]
    fn is_after_handles_mixed_precision() {
        // lexicographically "00Z" > "00.5Z" ('Z' > '.'), but temporally earlier
        assert_eq!(is_after("2026-06-04T12:00:00Z", "2026-06-04T12:00:00.500000Z"), Some(false));
        assert_eq!(is_after("2026-06-04T12:00:01Z", "2026-06-04T12:00:00.500000Z"), Some(true));
        assert_eq!(is_after("not a time", "2026-06-04T12:00:00Z"), None);
    }
}
