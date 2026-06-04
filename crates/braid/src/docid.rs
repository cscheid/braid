//! An opaque wrapper for the skein document id (strand br-redact-doc-id).
//!
//! The doc id is an **irrevocable read/write bearer capability**: anyone
//! holding it controls the skein forever, and the only post-leak remedy is
//! rotation. The most likely leak vectors are mundane — error messages in
//! CI logs, status lines in agent transcripts — so the id is guarded by
//! the type system rather than by discipline:
//!
//! - **no `Display` impl**: `format!("{doc_id}")` is a *compile error*
//! - **no `Serialize` impl**: it cannot ride along in `--json` output
//! - `Debug` prints only the redacted prefix
//! - [`DocId::redacted`] gives a six-character prefix for diagnostics —
//!   enough to tell two skeins apart, useless as a capability
//! - [`DocId::expose_secret`] is the single, greppable, affirmative way
//!   to the full value (named after the `secrecy` crate's convention).
//!   Legitimate call sites: parsing into samod's `DocumentId`, writing
//!   `.braid.toml`, and the explicit `braid secret` command.
//!
//! Note that samod's own `DocumentId` implements `Display` with the full
//! id; the companion discipline is that `DocumentId` values stay internal
//! plumbing and never reach user-facing strings.

#[derive(Clone, PartialEq, Eq)]
pub struct DocId(Box<str>);

impl DocId {
    pub fn new(id: impl Into<Box<str>>) -> Self {
        Self(id.into())
    }

    /// A six-character prefix plus an ellipsis: disambiguates skeins in
    /// error messages without granting the capability.
    pub fn redacted(&self) -> String {
        let prefix: String = self.0.chars().take(6).collect();
        format!("{prefix}…")
    }

    /// The full bearer capability. Every call site is a deliberate
    /// disclosure decision; `grep -rn expose_secret` is the audit.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for DocId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DocId({})", self.redacted())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_redacted_never_show_the_full_id() {
        let id = DocId::new("2An1tG1Fgqj43ViBvaD6UWfVrKZ6");
        assert_eq!(id.redacted(), "2An1tG…");
        assert_eq!(format!("{id:?}"), "DocId(2An1tG…)");
        assert!(!format!("{id:?}").contains("Fgqj"));
    }

    #[test]
    fn expose_secret_returns_the_capability() {
        let id = DocId::new("2An1tG1Fgqj43ViBvaD6UWfVrKZ6");
        assert_eq!(id.expose_secret(), "2An1tG1Fgqj43ViBvaD6UWfVrKZ6");
    }

    #[test]
    fn short_ids_redact_without_panicking() {
        assert_eq!(DocId::new("abc").redacted(), "abc…");
    }
}
