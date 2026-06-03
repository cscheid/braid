//! Local document cache (design decision D9).
//!
//! The cache is samod filesystem storage under an XDG-style directory, with
//! one twist: the first component of every storage key is the **document
//! id — a bearer secret** — and stock adapters splay it straight into
//! directory names. [`HashedKeyStorage`] maps the first key component
//! through SHA-256 before delegating, so the secret never appears on disk.
//!
//! The mapping is one-way, which is fine for `load`/`put`/`delete` (no keys
//! come back) but `load_range` must return keys the caller can interpret:
//! we restore the original first component from the *query prefix*. This is
//! sound because samod only ever calls `load_range` with prefixes of the
//! form `[doc_id, "snapshot"|"incremental"]` (verified against samod-core
//! 0.10; see `actors/document/load.rs`). An empty prefix cannot be
//! un-mapped, so it fails closed (returns nothing) rather than returning
//! corrupted keys.
//!
//! Cache layout: `<cache root>/braid/store/...`, where the cache root is
//! `BRAID_CACHE_DIR` > `XDG_CACHE_HOME` > `~/.cache`. The `braid/`
//! directory is created mode 700: contents are plaintext issue data, gated
//! by directory permissions (at-rest encryption is tracked as deferred
//! hardening in the design doc).

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};

use samod::storage::{Storage, StorageKey, TokioFilesystemStorage};
use sha2::{Digest, Sha256};

/// Hex SHA-256 of a key component.
pub fn hash_component(component: &str) -> String {
    let digest = Sha256::digest(component.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Replace the first component of `key` with its hash. Empty keys pass
/// through unchanged.
fn transform(key: &StorageKey) -> StorageKey {
    let mut parts: Vec<String> = key.into_iter().cloned().collect();
    if let Some(first) = parts.first_mut() {
        *first = hash_component(first);
    }
    // hex output contains no '/' or empty components, so this cannot fail
    StorageKey::from_parts(parts).expect("hashed components are always valid")
}

/// Replace the first component of `key` with `original` (the inverse of
/// [`transform`], given knowledge of the original from the query prefix).
fn restore(key: &StorageKey, original: &str) -> StorageKey {
    let mut parts: Vec<String> = key.into_iter().cloned().collect();
    if let Some(first) = parts.first_mut() {
        *first = original.to_string();
    }
    StorageKey::from_parts(parts).expect("restored components are always valid")
}

/// A [`Storage`] adapter that hashes the first key component (the document
/// id) before delegating to the inner storage.
#[derive(Debug, Clone)]
pub struct HashedKeyStorage<S> {
    inner: S,
}

impl<S> HashedKeyStorage<S> {
    pub fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S: Storage + Sync> Storage for HashedKeyStorage<S> {
    fn load(&self, key: StorageKey) -> impl Future<Output = Option<Vec<u8>>> + Send {
        let inner = self.inner.clone();
        async move { inner.load(transform(&key)).await }
    }

    fn load_range(
        &self,
        prefix: StorageKey,
    ) -> impl Future<Output = HashMap<StorageKey, Vec<u8>>> + Send {
        let inner = self.inner.clone();
        async move {
            let Some(original) = (&prefix).into_iter().next().cloned() else {
                // An empty prefix would return keys we cannot un-map; fail
                // closed (samod never issues such queries — see module docs).
                // No panic: this runs inside samod's storage task.
                return HashMap::new();
            };
            inner
                .load_range(transform(&prefix))
                .await
                .into_iter()
                .map(|(k, v)| (restore(&k, &original), v))
                .collect()
        }
    }

    fn put(&self, key: StorageKey, data: Vec<u8>) -> impl Future<Output = ()> + Send {
        let inner = self.inner.clone();
        async move { inner.put(transform(&key), data).await }
    }

    fn delete(&self, key: StorageKey) -> impl Future<Output = ()> + Send {
        let inner = self.inner.clone();
        async move { inner.delete(transform(&key)).await }
    }
}

/// Resolve the braid cache directory: `BRAID_CACHE_DIR` >
/// `XDG_CACHE_HOME/braid` > `~/.cache/braid`.
pub fn cache_dir(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let non_blank = |k: &str| {
        env(k).and_then(|v| {
            let t = v.trim().to_string();
            if t.is_empty() { None } else { Some(t) }
        })
    };
    if let Some(dir) = non_blank("BRAID_CACHE_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(xdg) = non_blank("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg).join("braid"));
    }
    non_blank("HOME").map(|home| PathBuf::from(home).join(".cache").join("braid"))
}

/// Create (mode 700) the cache directory and open hashed filesystem
/// storage rooted at `<dir>/store`.
pub fn open_cache_storage(
    dir: &Path,
) -> std::io::Result<HashedKeyStorage<TokioFilesystemStorage>> {
    let store = dir.join("store");
    std::fs::create_dir_all(&store)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for d in [dir, store.as_path()] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(HashedKeyStorage::new(TokioFilesystemStorage::new(store)))
}
