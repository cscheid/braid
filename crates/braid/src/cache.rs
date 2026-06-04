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

use samod::storage::{Storage, StorageKey};
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

// ---------------------------------------------------------------------------
// filesystem storage
// ---------------------------------------------------------------------------

/// Filesystem storage in samod's nodefs-compatible layout, hardened for
/// braid's many-short-lived-processes reality. samod's own
/// `TokioFilesystemStorage` (0.10) conflates IO errors with absence
/// (`metadata(..).ok()?`), so a transient failure under load makes a
/// document look missing — which braid then misreports as "not in the
/// local cache" (strand br-cache-flake). This implementation instead:
///
/// - treats only `NotFound` (and directory-shaped paths) as absence
/// - retries transient errors with backoff; *persistent* errors panic
///   with the path and error, never masquerade as absence
/// - writes atomically (temp file + rename) so a concurrent braid
///   process can never observe a half-written chunk
#[derive(Debug, Clone)]
pub struct FsStorage {
    root: PathBuf,
}

impl FsStorage {
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self { root: root.as_ref().to_path_buf() }
    }

    /// nodefs layout: the first key component is splayed into
    /// `<first two chars>/<rest>`; remaining components are plain segments.
    fn path_for(&self, key: &StorageKey) -> PathBuf {
        let mut path = self.root.clone();
        for (index, component) in key.into_iter().enumerate() {
            if index == 0 {
                let first_two: String = component.chars().take(2).collect();
                let remaining: String = component.chars().skip(2).collect();
                path.push(first_two);
                path.push(remaining);
            } else {
                path.push(component);
            }
        }
        path
    }
}

/// Retry `op` on transient IO errors. `NotFound` is returned immediately
/// (it is an answer, not a failure); after the retry budget the last error
/// propagates for the caller to report loudly.
async fn retry_io<T, F, Fut>(op: F) -> std::io::Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = std::io::Result<T>>,
{
    let mut delay = std::time::Duration::from_millis(10);
    let mut last_err = None;
    for _ in 0..5 {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(e),
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
        }
    }
    Err(last_err.expect("retry loop ran at least once"))
}

fn is_not_found(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound
}

impl Storage for FsStorage {
    fn load(&self, key: StorageKey) -> impl Future<Output = Option<Vec<u8>>> + Send {
        let path = self.path_for(&key);
        async move {
            match retry_io(|| tokio::fs::read(&path)).await {
                Ok(data) => Some(data),
                Err(e) if is_not_found(&e) => None,
                // a directory where a file should be = key has no value
                Err(e) if path.is_dir() => {
                    let _ = e;
                    None
                }
                Err(e) => panic!("braid cache: cannot read {}: {e}", path.display()),
            }
        }
    }

    fn load_range(
        &self,
        prefix: StorageKey,
    ) -> impl Future<Output = HashMap<StorageKey, Vec<u8>>> + Send {
        let root = self.path_for(&prefix);
        async move {
            let mut result = HashMap::new();
            if !root.is_dir() {
                return result; // genuinely absent prefix
            }
            let mut to_visit = vec![(root, prefix)];
            while let Some((dir, key_prefix)) = to_visit.pop() {
                let mut entries = match retry_io(|| tokio::fs::read_dir(&dir)).await {
                    Ok(entries) => entries,
                    Err(e) if is_not_found(&e) => continue, // raced a delete
                    Err(e) => panic!("braid cache: cannot list {}: {e}", dir.display()),
                };
                loop {
                    let entry = match entries.next_entry().await {
                        Ok(Some(entry)) => entry,
                        Ok(None) => break,
                        Err(e) => {
                            panic!("braid cache: cannot list {}: {e}", dir.display())
                        }
                    };
                    let Some(name) = entry.file_name().to_str().map(String::from) else {
                        continue; // non-UTF8 names are never braid's
                    };
                    // skip in-flight atomic writes
                    if name.starts_with(".tmp-") {
                        continue;
                    }
                    let Ok(key) = key_prefix.with_component(name) else {
                        continue;
                    };
                    let path = entry.path();
                    if path.is_dir() {
                        to_visit.push((path, key));
                    } else {
                        match retry_io(|| tokio::fs::read(&path)).await {
                            Ok(data) => {
                                result.insert(key, data);
                            }
                            Err(e) if is_not_found(&e) => {} // raced a delete
                            Err(e) => panic!("braid cache: cannot read {}: {e}", path.display()),
                        }
                    }
                }
            }
            result
        }
    }

    fn put(&self, key: StorageKey, data: Vec<u8>) -> impl Future<Output = ()> + Send {
        let path = self.path_for(&key);
        async move {
            let parent = path.parent().expect("storage paths always have a parent");
            if let Err(e) = retry_io(|| tokio::fs::create_dir_all(parent)).await {
                panic!("braid cache: cannot create {}: {e}", parent.display());
            }
            // atomic write: temp file in the same directory, then rename,
            // so concurrent readers never see a partial chunk
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let tmp = parent.join(format!(
                ".tmp-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            if let Err(e) = retry_io(|| tokio::fs::write(&tmp, &data)).await {
                panic!("braid cache: cannot write {}: {e}", tmp.display());
            }
            if let Err(e) = retry_io(|| tokio::fs::rename(&tmp, &path)).await {
                panic!("braid cache: cannot rename into {}: {e}", path.display());
            }
        }
    }

    fn delete(&self, key: StorageKey) -> impl Future<Output = ()> + Send {
        let path = self.path_for(&key);
        async move {
            match retry_io(|| tokio::fs::remove_file(&path)).await {
                Ok(()) => {}
                Err(e) if is_not_found(&e) => {}
                Err(e) => panic!("braid cache: cannot delete {}: {e}", path.display()),
            }
        }
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
pub fn open_cache_storage(dir: &Path) -> std::io::Result<HashedKeyStorage<FsStorage>> {
    let store = dir.join("store");
    std::fs::create_dir_all(&store)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for d in [dir, store.as_path()] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(HashedKeyStorage::new(FsStorage::new(store)))
}
