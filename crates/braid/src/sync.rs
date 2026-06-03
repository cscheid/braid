//! Per-invocation sync (design decision D2).
//!
//! Each command dials the **one configured sync server**, exchanges sync
//! messages bounded by a timeout, and exits. There is no daemon: users who
//! want lower latency run a local relay (itself a sync server) and point
//! braid at it.
//!
//! Offline is a first-class outcome, not an error: when the server cannot
//! be reached within the timeout, commands fall back to the local cache
//! and say so on stderr. Only the explicit `braid sync` command treats
//! offline as a failure.

use std::time::Duration;

use anyhow::{Result, bail};
use samod::{BackoffConfig, ConnectionId, DialerHandle, Repo, Url};

/// Sync timeout: `BRAID_SYNC_TIMEOUT` (seconds, fractional ok), default 5s.
pub fn sync_timeout() -> Duration {
    std::env::var("BRAID_SYNC_TIMEOUT")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|s| *s > 0.0)
        .map(Duration::from_secs_f64)
        .unwrap_or(Duration::from_secs(5))
}

pub enum Connect {
    Connected { dialer: DialerHandle, conn: ConnectionId },
    /// The server could not be reached within the timeout. Carries a
    /// human-readable reason.
    Offline(String),
}

/// Dial `server_url` (schemes: `ws`, `wss`, `tcp`) and wait for an
/// established connection, bounded by `timeout`.
///
/// A malformed or unsupported URL is a hard error (it's a configuration
/// mistake); an unreachable server is `Connect::Offline`.
pub async fn connect(repo: &Repo, server_url: &str, timeout: Duration) -> Result<Connect> {
    let url = Url::parse(server_url)
        .map_err(|e| anyhow::anyhow!("invalid sync_server URL {server_url:?}: {e}"))?;

    // Bounded retries inside the window; the outer timeout is the real cap.
    let backoff = BackoffConfig {
        initial_delay: Duration::from_millis(100),
        max_delay: Duration::from_secs(1),
        max_retries: None,
    };

    let dialer = match url.scheme() {
        "ws" | "wss" => repo
            .dial_websocket(url, backoff)
            .map_err(|_| anyhow::anyhow!("samod repo stopped unexpectedly"))?,
        "tcp" => repo
            .dial_tcp(url, backoff)
            .map_err(|e| anyhow::anyhow!("cannot dial {server_url}: {e}"))?,
        other => bail!(
            "unsupported sync_server scheme {other:?} in {server_url:?} \
             (supported: wss://, ws://, tcp://)"
        ),
    };

    match tokio::time::timeout(timeout, dialer.established()).await {
        Ok(Ok(_peer_info)) => match dialer.connection_id() {
            Some(conn) => Ok(Connect::Connected { dialer, conn }),
            None => {
                dialer.close();
                Ok(Connect::Offline("connection lost immediately after establishing".into()))
            }
        },
        Ok(Err(failed)) => {
            dialer.close();
            Ok(Connect::Offline(format!("could not connect to {server_url}: {failed:?}")))
        }
        Err(_elapsed) => {
            // Stop retrying: this invocation is now offline.
            dialer.close();
            Ok(Connect::Offline(format!(
                "could not reach {server_url} within {timeout:?}"
            )))
        }
    }
}
