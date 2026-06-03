//! Opening a tracker: config → cache storage → samod repo → DocHandle.
//!
//! Phase 1 is local-only: the repo has no dialers, so `find` resolves
//! purely from the cache. Phase 2 adds the per-invocation dial/sync here.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use samod::{DocHandle, DocumentId, Repo};

use crate::cache;
use crate::config::{self, ResolvedConfig};

pub struct OpenedTracker {
    pub cfg: ResolvedConfig,
    pub repo: Repo,
    pub doc: DocHandle,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

/// Open the braid cache as samod storage.
pub fn open_cache() -> Result<cache::HashedKeyStorage<samod::storage::TokioFilesystemStorage>> {
    let dir = cache::cache_dir(&env)
        .context("cannot determine a cache directory (HOME is not set)")?;
    cache::open_cache_storage(&dir)
        .with_context(|| format!("cannot open braid cache at {}", dir.display()))
}

/// Build a samod repo over the cache, with no network connections.
pub async fn open_repo() -> Result<Repo> {
    Ok(Repo::build_tokio().with_storage(open_cache()?).load().await)
}

/// Resolve config from `cwd` and load its tracker document from the cache.
pub async fn open_tracker(cwd: &Path) -> Result<OpenedTracker> {
    let cfg = config::load(cwd)?;
    let doc_id: DocumentId = cfg.doc_id.parse().map_err(|e| {
        anyhow!(
            "configured doc_id {:?} is not a valid automerge document id: {e:?}",
            cfg.doc_id
        )
    })?;
    let repo = open_repo().await?;
    match repo.find(doc_id).await {
        Ok(Some(doc)) => Ok(OpenedTracker { cfg, repo, doc }),
        Ok(None) => bail!(
            "tracker document {} is not in the local cache.\n\
             This machine has not synced it yet — `braid sync` (Phase 2) will \
             fetch it from the sync server — or the doc_id is wrong.",
            cfg.doc_id
        ),
        Err(_) => bail!("samod repo stopped unexpectedly"),
    }
}
