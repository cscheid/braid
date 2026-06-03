//! Timestamp helpers. Timestamps in the schema are writer-set RFC 3339
//! strings (design decision D10): trivially JSON-exportable and independent
//! of automerge's history.

use chrono::{SecondsFormat, Utc};

/// Current time as an RFC 3339 UTC string with microsecond precision,
/// e.g. `2026-06-03T14:37:32.946678Z` (matches the beads JSONL style).
pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true)
}
