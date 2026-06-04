//! Opening a skein: config → cache storage → samod repo → dial → DocHandle.
//!
//! Per design decision D2, every command syncs per-invocation: dial the
//! configured server (bounded by a timeout), exchange, exit. When the
//! server is unreachable, commands fall back to the local cache and warn.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use samod::{ConnectionId, DocHandle, DocumentId, Repo};

use crate::cache;
use crate::config::{self, ResolvedConfig};
use crate::sync::{Connect, connect, sync_timeout};

pub struct OpenedSkein {
    pub cfg: ResolvedConfig,
    pub repo: Repo,
    pub doc: DocHandle,
    /// Established server connection, if any.
    pub conn: Option<ConnectionId>,
    /// Why we're offline, when we are.
    pub offline_reason: Option<String>,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

/// Open the braid cache as samod storage.
pub fn open_cache() -> Result<cache::HashedKeyStorage<cache::FsStorage>> {
    let dir = cache::cache_dir(&env)
        .context("cannot determine a cache directory (HOME is not set)")?;
    cache::open_cache_storage(&dir)
        .with_context(|| format!("cannot open braid cache at {}", dir.display()))
}

/// Build a samod repo over the cache, with no network connections (used by
/// `init`, which creates the document before any secret exists on disk).
///
/// `BRAID_NO_CACHE=1` selects in-memory storage instead: fully stateless
/// invocations that fetch everything from the server and persist nothing
/// (D9). Useless offline, by construction.
pub async fn open_repo() -> Result<Repo> {
    let no_cache = std::env::var("BRAID_NO_CACHE").is_ok_and(|v| {
        let v = v.trim();
        !v.is_empty() && v != "0"
    });
    if no_cache {
        Ok(Repo::build_tokio()
            .with_storage(samod::storage::InMemoryStorage::new())
            .load()
            .await)
    } else {
        Ok(Repo::build_tokio().with_storage(open_cache()?).load().await)
    }
}

/// Resolve config from `cwd`, dial the configured server (offline
/// tolerated), load the skein document — from the cache or, failing that,
/// from the server — pull the latest changes, and **refuse a rotated
/// skein** (design D-R4: every command stops writing to a dead document).
pub async fn open_skein(cwd: &Path) -> Result<OpenedSkein> {
    let opened = open_skein_unchecked(cwd).await?;
    let meta = opened.doc.with_document(|d| braid_core::amdoc::hydrate_metadata(d))?;
    if let Some(rotated_at) = &meta.rotated_at {
        if meta.rotated_to.is_some() {
            bail!(
                "this skein was rotated on {rotated_at}: a successor document \
                 holds the live state.\nRun `braid rotate --adopt` to switch \
                 this clone to the new skein."
            );
        } else {
            bail!(
                "this skein was rotated on {rotated_at} with revocation: the \
                 successor id was deliberately not recorded here.\nObtain the \
                 new secret out-of-band (`braid secret` on an up-to-date \
                 machine) and update this clone's configuration."
            );
        }
    }
    Ok(opened)
}

/// [`open_skein`] without the rotation refusal (and pulls all the same).
/// Used by `braid rotate --adopt`, which must open the *old* document to
/// read its forwarding record.
pub async fn open_skein_unchecked(cwd: &Path) -> Result<OpenedSkein> {
    let cfg = config::load(cwd)?;
    let doc_id: DocumentId = cfg.doc_id.expose_secret().parse().map_err(|e| {
        anyhow!(
            "configured doc_id ({}) is not a valid automerge document id: {e:?}",
            cfg.doc_id.redacted()
        )
    })?;
    let repo = open_repo().await?;

    let (conn, offline_reason) = match connect(&repo, &cfg.sync_server, sync_timeout()).await? {
        Connect::Connected { conn, .. } => (Some(conn), None),
        Connect::Offline(reason) => {
            eprintln!("braid: offline ({reason}); using the local cache");
            (None, Some(reason))
        }
    };

    // With an established connection, `find` asks the server for documents
    // missing from the cache.
    match repo.find(doc_id).await {
        Ok(Some(doc)) => {
            let opened = OpenedSkein { cfg, repo, doc, conn, offline_reason };
            // Every command wants the freshest state (and the rotation
            // check must see the latest metadata), so pull here.
            opened.pull().await;
            Ok(opened)
        }
        Ok(None) => {
            if conn.is_some() {
                bail!(
                    "skein {} was not found in the local cache, and {} \
                     does not have it either.\nCheck the doc_id (`braid secret` shows \
                     it), or run `braid sync` from a machine that has the skein.",
                    cfg.doc_id.redacted(),
                    cfg.sync_server
                )
            } else {
                bail!(
                    "skein {} is not in the local cache and the sync \
                     server is unreachable.\nReconnect and retry, or check the doc_id.",
                    cfg.doc_id.redacted()
                )
            }
        }
        Err(_) => bail!("samod repo stopped unexpectedly"),
    }
}

impl OpenedSkein {
    /// Wait (bounded) until everything the server has is local. No-op when
    /// offline. Call before reading.
    pub async fn pull(&self) {
        if let Some(conn) = self.conn {
            let _ = tokio::time::timeout(sync_timeout(), self.doc.we_have_their_changes(conn))
                .await;
        }
    }

    /// Wait (bounded) until the server has everything local, then shut the
    /// repo down (flushing the cache). Call after writing.
    pub async fn push_and_close(self) {
        if let Some(conn) = self.conn {
            let confirmed = tokio::time::timeout(
                sync_timeout(),
                self.doc.they_have_our_changes(conn),
            )
            .await
            .is_ok();
            if !confirmed {
                eprintln!(
                    "braid: changes saved locally, but the server did not confirm \
                     receipt in time; run `braid sync` later to be sure"
                );
            }
        }
        self.repo.stop().await;
    }

    /// Shut down without a write barrier (flushes the cache). Call after
    /// read-only commands.
    pub async fn close(self) {
        self.repo.stop().await;
    }
}
