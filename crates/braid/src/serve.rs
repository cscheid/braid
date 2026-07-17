//! `braid serve` — run a loom: a standalone samod sync server that braid
//! clients (and any other automerge-repo peer) can collaborate through
//! when the default sync server is unreachable or unwanted (issue #22).
//!
//! The loom is deliberately minimal: a samod repo, a TCP listener, and a
//! websocket handshake per connection, reusing the same
//! [`crate::ws::ws_transport`] glue the dialer uses (and for the same
//! reason: samod's own `tungstenite` feature hardcodes native-tls, which
//! breaks static musl builds — br-f3b18xoa). There is no HTTP surface, no
//! TLS (terminate `wss://` in a reverse proxy), and no auth: exactly like
//! the public sync server, possession of a doc id grants read/write access
//! to that document, so the loom binds loopback unless told otherwise.
//!
//! Two properties matter beyond the happy path:
//!
//! - **Announce policy**: [`NeverAnnounce`]. samod's default
//!   (`AlwaysAnnounce`) would announce every document the loom is
//!   synchronizing to every connected peer — leaking doc ids, which are
//!   bearer capabilities, across tenants. The loom only ever responds to
//!   requests for documents the peer already knows by id.
//! - **Storage keys**: with `--data-dir`, documents persist through the
//!   same [`crate::cache::HashedKeyStorage`] the client cache uses, so doc
//!   ids never appear on the loom's disk (design decision D-serve-2).

use std::path::PathBuf;

use anyhow::{Context, Result};
use futures::StreamExt;
use samod::{AcceptorEvent, NeverAnnounce, Repo};

use crate::cache;

pub struct ServeOpts {
    pub host: String,
    pub port: u16,
    pub data_dir: Option<PathBuf>,
    pub in_memory: bool,
}

pub async fn serve(opts: ServeOpts) -> Result<()> {
    let repo = match &opts.data_dir {
        Some(dir) => {
            let storage = cache::open_cache_storage(dir)
                .with_context(|| format!("cannot open loom storage at {}", dir.display()))?;
            Repo::build_tokio()
                .with_storage(storage)
                .with_announce_policy(NeverAnnounce)
                .load()
                .await
        }
        None => {
            debug_assert!(opts.in_memory, "clap requires one of --data-dir / --in-memory");
            Repo::build_tokio()
                .with_storage(samod::storage::InMemoryStorage::new())
                .with_announce_policy(NeverAnnounce)
                .load()
                .await
        }
    };

    let listener = tokio::net::TcpListener::bind((opts.host.as_str(), opts.port))
        .await
        .with_context(|| format!("cannot bind {}:{}", opts.host, opts.port))?;
    let addr = listener.local_addr().context("cannot determine the bound address")?;
    let url = format!("ws://{addr}");

    let acceptor = repo
        .make_acceptor(samod::Url::parse(&url).expect("bound socket addresses form valid URLs"))
        .map_err(|_| anyhow::anyhow!("samod repo stopped before the loom could listen"))?;

    // The one machine-readable line, on stdout: tests and scripts parse
    // the URL out of it. Everything else goes to stderr.
    println!("loom listening on {url}");
    match &opts.data_dir {
        Some(dir) => eprintln!("loom: persisting skeins under {}", dir.display()),
        None => eprintln!("loom: in-memory only; skeins are forgotten on exit"),
    }
    eprintln!("loom: point clients here with BRAID_SYNC_URL={url} (or sync_server in .braid.toml)");
    eprintln!("Press Ctrl-C to stop.");

    // events() borrows the handle (samod 0.10 overcaptures under Rust
    // 2024 impl-Trait rules), so give the logging task its own clone.
    let event_acceptor = acceptor.clone();
    let event_log = tokio::spawn(async move {
        let mut events = event_acceptor.events();
        while let Some(event) = events.next().await {
            match event {
                AcceptorEvent::ClientConnected { peer_info, connection_id } => {
                    eprintln!(
                        "loom: peer {} connected (connection {connection_id:?})",
                        peer_info.peer_id
                    );
                }
                AcceptorEvent::ClientDisconnected { connection_id, reason } => {
                    eprintln!("loom: connection {connection_id:?} closed: {reason}");
                }
            }
        }
    });

    let accept_loop = async {
        loop {
            match listener.accept().await {
                Ok((stream, remote)) => {
                    let acceptor = acceptor.clone();
                    // Handshake per-task so one slow client cannot stall
                    // the accept loop.
                    tokio::spawn(async move {
                        match tokio_tungstenite::accept_async(stream).await {
                            Ok(ws) => {
                                let _ = acceptor.accept(crate::ws::ws_transport(ws));
                            }
                            Err(e) => {
                                eprintln!("loom: websocket handshake with {remote} failed: {e}");
                            }
                        }
                    });
                }
                Err(e) => eprintln!("loom: accept error: {e}"),
            }
        }
    };

    tokio::select! {
        _ = accept_loop => {}
        _ = shutdown_signal() => {}
    }

    eprintln!("\nloom: shutting down");
    event_log.abort();
    // Flushes pending storage writes before returning.
    repo.stop().await;
    eprintln!("loom: stopped.");
    Ok(())
}

/// Resolves on Ctrl-C (SIGINT) and, on Unix, SIGTERM — what launchd,
/// systemd, and process supervisors send — so the repo always gets a
/// clean stop() and storage flush.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("cannot install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
