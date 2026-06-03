//! Issue and comment id generation (design decision D6).
//!
//! Ids are `<prefix>-[slug-]<suffix>` where `suffix` is [`ID_SUFFIX_LEN`]
//! random base36 characters. There is no uniqueness oracle in a CRDT
//! world — collision safety comes from entropy: 36^8 ≈ 2.8e12, so even
//! 10⁴ issues created across replicas put the collision probability
//! around 2e-5 (birthday bound). The merge tests pin what happens on the
//! improbable collision: the document still converges, one object wins.
//!
//! Ids never contain `:` (the dependency map key separator) — enforced by
//! the slug normalizer and the base36 alphabet.

use rand::Rng;

/// Number of random base36 characters in a generated id suffix.
pub const ID_SUFFIX_LEN: usize = 8;

/// Maximum length of a normalized slug segment (matches beads).
pub const MAX_SLUG_LEN: usize = 48;

const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn random_suffix(rng: &mut impl Rng) -> String {
    (0..ID_SUFFIX_LEN)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

/// Generate an issue id: `<prefix>-<suffix>` or `<prefix>-<slug>-<suffix>`.
/// The slug, if given, is normalized via [`normalize_slug`]; a slug that
/// normalizes to nothing is dropped.
pub fn new_issue_id_with(rng: &mut impl Rng, prefix: &str, slug: Option<&str>) -> String {
    let suffix = random_suffix(rng);
    match slug.and_then(normalize_slug) {
        Some(slug) => format!("{prefix}-{slug}-{suffix}"),
        None => format!("{prefix}-{suffix}"),
    }
}

/// [`new_issue_id_with`] using the thread-local RNG.
pub fn new_issue_id(prefix: &str, slug: Option<&str>) -> String {
    new_issue_id_with(&mut rand::rng(), prefix, slug)
}

/// Generate a comment id: `c-<suffix>`.
pub fn new_comment_id_with(rng: &mut impl Rng) -> String {
    format!("c-{}", random_suffix(rng))
}

/// [`new_comment_id_with`] using the thread-local RNG.
pub fn new_comment_id() -> String {
    new_comment_id_with(&mut rand::rng())
}

/// Normalize a human-supplied slug: lowercase; non-alphanumeric runs
/// become single hyphens; leading/trailing hyphens trimmed; truncated to
/// [`MAX_SLUG_LEN`] without leaving a trailing hyphen. Returns `None` if
/// nothing survives.
pub fn normalize_slug(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len().min(MAX_SLUG_LEN));
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    // The slug is pure ASCII by construction, so byte truncation is safe.
    out.truncate(MAX_SLUG_LEN);
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { None } else { Some(out) }
}
