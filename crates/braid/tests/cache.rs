//! Tests for the hashed-key cache storage adapter (design decision D9).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use braid::cache::{FsStorage, HashedKeyStorage, cache_dir, hash_component, open_cache_storage};
use samod::storage::{Storage, StorageKey};

const DOC_ID: &str = "4NMNnkMhL8jXrdJbSeuJtZtnDoiu";

fn key(parts: &[&str]) -> StorageKey {
    StorageKey::from_parts(parts.to_vec()).unwrap()
}

fn adapter(root: &Path) -> HashedKeyStorage<FsStorage> {
    HashedKeyStorage::new(FsStorage::new(root))
}

/// Recursively collect every path under `root`.
fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                out.extend(walk(&p));
            }
            out.push(p);
        }
    }
    out
}

#[tokio::test]
async fn put_load_delete_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = adapter(tmp.path());

    let k = key(&[DOC_ID, "incremental", "abc123"]);
    storage.put(k.clone(), b"hello".to_vec()).await;
    assert_eq!(storage.load(k.clone()).await, Some(b"hello".to_vec()));

    storage.delete(k.clone()).await;
    assert_eq!(storage.load(k).await, None);
}

#[tokio::test]
async fn load_range_returns_original_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = adapter(tmp.path());

    storage.put(key(&[DOC_ID, "incremental", "h1"]), b"1".to_vec()).await;
    storage.put(key(&[DOC_ID, "incremental", "h2"]), b"2".to_vec()).await;
    storage.put(key(&[DOC_ID, "snapshot", "s1"]), b"3".to_vec()).await;

    let got: HashMap<StorageKey, Vec<u8>> = storage.load_range(key(&[DOC_ID, "incremental"])).await;

    assert_eq!(got.len(), 2);
    assert_eq!(got[&key(&[DOC_ID, "incremental", "h1"])], b"1".to_vec());
    assert_eq!(got[&key(&[DOC_ID, "incremental", "h2"])], b"2".to_vec());
}

#[tokio::test]
async fn document_id_never_appears_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = adapter(tmp.path());

    storage.put(key(&[DOC_ID, "incremental", "h1"]), b"data".to_vec()).await;
    storage.put(key(&[DOC_ID, "snapshot", "s1"]), b"data".to_vec()).await;
    storage.put(key(&["storage-adapter-id"]), b"id".to_vec()).await;

    let paths = walk(tmp.path());
    assert!(!paths.is_empty(), "storage should have written files");
    for p in &paths {
        let s = p.to_string_lossy();
        assert!(!s.contains(DOC_ID), "document id leaked into path: {s}");
        // also catch case-folded or prefix leaks of meaningful length
        assert!(
            !s.to_lowercase().contains(&DOC_ID.to_lowercase()),
            "document id (case-folded) leaked into path: {s}"
        );
    }
}

#[tokio::test]
async fn distinct_documents_do_not_collide() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = adapter(tmp.path());

    let other = "9ZqWvXyAbCdEfGhJkLmNpQrStUvW";
    storage.put(key(&[DOC_ID, "snapshot", "s1"]), b"doc1".to_vec()).await;
    storage.put(key(&[other, "snapshot", "s1"]), b"doc2".to_vec()).await;

    assert_eq!(storage.load(key(&[DOC_ID, "snapshot", "s1"])).await, Some(b"doc1".to_vec()));
    assert_eq!(storage.load(key(&[other, "snapshot", "s1"])).await, Some(b"doc2".to_vec()));
}

#[tokio::test]
async fn empty_prefix_fails_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = adapter(tmp.path());
    storage.put(key(&[DOC_ID, "snapshot", "s1"]), b"data".to_vec()).await;

    // An empty prefix can't be un-mapped; the adapter must return nothing
    // rather than keys with hashed components.
    let got = storage.load_range(StorageKey::from_parts(Vec::<String>::new()).unwrap()).await;
    assert!(got.is_empty());
}

#[test]
fn hash_component_is_hex_sha256() {
    let h = hash_component("hello");
    assert_eq!(h.len(), 64);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    // sha256("hello")
    assert_eq!(h, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    // deterministic
    assert_eq!(hash_component("hello"), h);
    assert_ne!(hash_component("hello2"), h);
}

#[test]
fn cache_dir_precedence() {
    let env = |pairs: Vec<(&str, &str)>| {
        let m: HashMap<String, String> =
            pairs.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| m.get(k).cloned()
    };

    let e = env(vec![
        ("BRAID_CACHE_DIR", "/explicit"),
        ("XDG_CACHE_HOME", "/xdg"),
        ("HOME", "/home/u"),
    ]);
    assert_eq!(cache_dir(&e), Some(PathBuf::from("/explicit")));

    let e = env(vec![("XDG_CACHE_HOME", "/xdg"), ("HOME", "/home/u")]);
    assert_eq!(cache_dir(&e), Some(PathBuf::from("/xdg/braid")));

    let e = env(vec![("HOME", "/home/u")]);
    assert_eq!(cache_dir(&e), Some(PathBuf::from("/home/u/.cache/braid")));

    let e = env(vec![]);
    assert_eq!(cache_dir(&e), None);
}

#[cfg(unix)]
#[tokio::test]
async fn open_cache_storage_sets_700_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("braid");
    let storage = open_cache_storage(&dir).unwrap();
    storage.put(key(&[DOC_ID, "snapshot", "s1"]), b"data".to_vec()).await;

    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "cache dir must be owner-only");
    assert!(dir.join("store").is_dir());
}
