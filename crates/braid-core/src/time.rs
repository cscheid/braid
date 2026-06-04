//! Timestamp helpers. Timestamps in the schema are writer-set RFC 3339
//! strings (design decision D10): trivially JSON-exportable and independent
//! of automerge's history.

use chrono::{SecondsFormat, Utc};

/// Current time as an RFC 3339 UTC string with microsecond precision,
/// e.g. `2026-06-03T14:37:32.946678Z` (matches the beads JSONL style).
pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true)
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

    #[test]
    fn is_after_handles_mixed_precision() {
        // lexicographically "00Z" > "00.5Z" ('Z' > '.'), but temporally earlier
        assert_eq!(is_after("2026-06-04T12:00:00Z", "2026-06-04T12:00:00.500000Z"), Some(false));
        assert_eq!(is_after("2026-06-04T12:00:01Z", "2026-06-04T12:00:00.500000Z"), Some(true));
        assert_eq!(is_after("not a time", "2026-06-04T12:00:00Z"), None);
    }
}
