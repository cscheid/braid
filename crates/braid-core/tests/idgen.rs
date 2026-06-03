//! Tests for issue/comment id generation (design decision D6).

use std::collections::HashSet;

use braid_core::id::{
    ID_SUFFIX_LEN, MAX_SLUG_LEN, new_comment_id_with, new_issue_id_with, normalize_slug,
};
use rand::{SeedableRng, rngs::StdRng};

fn rng() -> StdRng {
    StdRng::seed_from_u64(0xb4a1d)
}

fn is_base36(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

#[test]
fn id_without_slug_has_prefix_dash_suffix() {
    let id = new_issue_id_with(&mut rng(), "br", None);
    let (prefix, suffix) = id.split_once('-').unwrap();
    assert_eq!(prefix, "br");
    assert_eq!(suffix.len(), ID_SUFFIX_LEN);
    assert!(is_base36(suffix), "suffix must be base36: {suffix:?}");
}

#[test]
fn id_with_slug_inserts_normalized_slug() {
    let id = new_issue_id_with(&mut rng(), "br", Some("Fix CRLF handling!"));
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(parts[0], "br");
    assert_eq!(&parts[1..parts.len() - 1], &["fix", "crlf", "handling"]);
    let suffix = parts.last().unwrap();
    assert_eq!(suffix.len(), ID_SUFFIX_LEN);
    assert!(is_base36(suffix));
}

#[test]
fn empty_or_degenerate_slug_is_dropped() {
    for raw in ["", "   ", "!!!", "---", "→🎉"] {
        let id = new_issue_id_with(&mut rng(), "br", Some(raw));
        let (prefix, suffix) = id.split_once('-').unwrap();
        assert_eq!(prefix, "br", "slug {raw:?} should be dropped entirely");
        assert_eq!(suffix.len(), ID_SUFFIX_LEN);
        assert!(is_base36(suffix));
    }
}

#[test]
fn slug_normalization_cases() {
    assert_eq!(normalize_slug("My Thing!"), Some("my-thing".into()));
    assert_eq!(normalize_slug("--a--b--"), Some("a-b".into()));
    assert_eq!(normalize_slug("UPPER_case.mixed"), Some("upper-case-mixed".into()));
    assert_eq!(normalize_slug("héllo wörld"), Some("h-llo-w-rld".into()));
    assert_eq!(normalize_slug("already-fine"), Some("already-fine".into()));
    assert_eq!(normalize_slug(""), None);
    assert_eq!(normalize_slug("!!!"), None);

    // truncation: stays within MAX_SLUG_LEN and never ends with '-'
    let long = "a ".repeat(60);
    let slug = normalize_slug(&long).unwrap();
    assert!(slug.len() <= MAX_SLUG_LEN);
    assert!(!slug.ends_with('-'));
    assert!(slug.starts_with("a-a-a"));
}

#[test]
fn ids_never_contain_colon_or_whitespace() {
    // ':' is the dependency map key separator; ids must never contain it.
    let mut r = rng();
    for raw_slug in [None, Some("with:colon and spaces"), Some("a:b:c")] {
        let id = new_issue_id_with(&mut r, "br", raw_slug);
        assert!(!id.contains(':'), "id must not contain ':': {id:?}");
        assert!(!id.contains(char::is_whitespace), "id must not contain whitespace: {id:?}");
    }
}

#[test]
fn seeded_generation_is_collision_free_at_scale() {
    // Deterministic (seeded) draw of 100k ids: any duplicate here would
    // indicate a broken generator, not bad luck (expected collisions at
    // 100k over 36^8 ≈ 0.0017; with a fixed seed the outcome is fixed and
    // this test is not flaky).
    let mut r = rng();
    let mut seen = HashSet::new();
    for _ in 0..100_000 {
        let id = new_issue_id_with(&mut r, "br", None);
        assert!(seen.insert(id.clone()), "duplicate id generated: {id}");
    }
}

#[test]
fn comment_ids_use_c_prefix() {
    let id = new_comment_id_with(&mut rng());
    let (prefix, suffix) = id.split_once('-').unwrap();
    assert_eq!(prefix, "c");
    assert_eq!(suffix.len(), ID_SUFFIX_LEN);
    assert!(is_base36(suffix));
}

#[test]
fn suffix_distribution_covers_alphabet() {
    // Sanity check that the generator uses the whole base36 alphabet
    // (catches off-by-one range bugs like 0..35 over a 36-char table).
    let mut r = rng();
    let mut seen_chars = HashSet::new();
    for _ in 0..2_000 {
        let id = new_issue_id_with(&mut r, "x", None);
        let (_, suffix) = id.split_once('-').unwrap();
        seen_chars.extend(suffix.chars());
    }
    assert_eq!(seen_chars.len(), 36, "all 36 alphabet characters should appear");
}
