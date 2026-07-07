//! WebSocket ↔ samod transport glue.
//!
//! samod's own `tungstenite` cargo feature hardcodes `native-tls`, which
//! on Linux drags in OpenSSL and breaks static musl release builds
//! (strand br-f3b18xoa). braid therefore talks to `tokio-tungstenite`
//! directly and hands samod a [`Transport`] built here. The same
//! conversion serves both directions: the dialer in [`crate::sync`] and
//! the in-process accept loop in the sync e2e tests.

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;
use futures::{SinkExt, StreamExt, TryStreamExt};
use rustls::pki_types::CertificateDer;
use samod::{Dialer, Transport, Url};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

/// Build a rustls root store that trusts, in this order:
///
/// 1. the compiled-in webpki (Mozilla) roots, so a static binary trusts
///    public sync servers even in a container with no system trust store
///    (the original motivation for br-f3b18xoa); plus
/// 2. every anchor in `extra` — the platform / sandbox / corporate
///    MITM-proxy CAs. Without these, a TLS-terminating egress proxy (as in
///    Claude Code's web sandbox) presents a certificate signed by a private
///    CA that the webpki roots don't know, and every `wss://` dial fails
///    with `UnknownIssuer`.
///
/// A malformed anchor in the system bundle is skipped rather than sinking
/// every dial.
fn root_store_with(
    extra: impl IntoIterator<Item = CertificateDer<'static>>,
) -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for cert in extra {
        let _ = roots.add(cert);
    }
    roots
}

/// Platform trust anchors. `rustls_native_certs` honors the OpenSSL
/// `SSL_CERT_FILE` / `SSL_CERT_DIR` environment variables and the system
/// store, which is how a sandbox or proxy CA bundle reaches braid. Load
/// errors are non-fatal: we still have the compiled-in webpki roots.
fn native_roots() -> Vec<CertificateDer<'static>> {
    rustls_native_certs::load_native_certs().certs
}

/// The shared client TLS config: webpki roots + platform CAs, built once.
///
/// `builder_with_provider(ring)` is explicit because braid enables rustls's
/// `ring` provider (not the default aws-lc-rs) and installs no process-wide
/// default provider, so `ClientConfig::builder()` would panic.
fn client_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = root_store_with(native_roots());
            let config = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .expect("ring provides every safe-default protocol version")
            .with_root_certificates(roots)
            .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

/// A [`Dialer`] for `ws://` and `wss://` endpoints, with TLS provided by
/// rustls trusting the compiled-in webpki roots plus the platform CA store
/// (see [`root_store_with`]).
///
/// The drop-in replacement for samod's own `TungsteniteDialer`, which
/// braid avoids because samod's `tungstenite` feature hardcodes
/// native-tls (strand br-f3b18xoa).
pub struct WsDialer {
    url: Url,
}

impl WsDialer {
    pub fn new(url: Url) -> Self {
        Self { url }
    }
}

impl Dialer for WsDialer {
    fn url(&self) -> Url {
        self.url.clone()
    }

    fn connect(
        &self,
    ) -> BoxFuture<'static, Result<Transport, Box<dyn std::error::Error + Send + Sync + 'static>>>
    {
        let url = self.url.clone();
        Box::pin(async move {
            let connector = tokio_tungstenite::Connector::Rustls(client_config());
            let (ws, _response) = tokio_tungstenite::connect_async_tls_with_config(
                url.as_str(),
                None,
                false,
                Some(connector),
            )
            .await?;
            Ok(ws_transport(ws))
        })
    }
}

/// Error type for the websocket↔bytes adaptation, mirroring samod's
/// internal `NetworkError`: the transport layer only needs `Display`.
#[derive(Debug)]
pub struct WsError(String);

impl std::fmt::Display for WsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for WsError {}

/// Convert an established websocket into a samod [`Transport`].
///
/// The samod sync protocol is binary-only: `Binary` frames pass through,
/// `Ping`/`Pong`/`Close` are protocol chatter handled by tungstenite and
/// filtered out, and a `Text` frame is a peer bug surfaced as an error.
pub fn ws_transport<S>(ws: WebSocketStream<S>) -> Transport
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let (sink, stream) = ws.split();

    let stream = stream
        .map_err(|e| WsError(format!("websocket receive error: {e}")))
        .try_filter_map(|msg| {
            futures::future::ready(match msg {
                Message::Binary(data) => Ok(Some(data.to_vec())),
                Message::Ping(_) | Message::Pong(_) | Message::Close(_) => Ok(None),
                Message::Text(_) => {
                    Err(WsError("unexpected text message on sync websocket".into()))
                }
                // Raw frames only surface when reading with
                // `read_frame`-style APIs, never from a message stream.
                Message::Frame(_) => unreachable!("unexpected raw frame message"),
            })
        })
        .boxed();

    let sink = sink
        .sink_map_err(|e| WsError(format!("websocket send error: {e}")))
        .with(|bytes: Vec<u8>| futures::future::ready(Ok::<_, WsError>(Message::binary(bytes))));

    Transport::new(stream, sink)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A self-signed CA that is deliberately NOT one of the webpki roots,
    /// standing in for a sandbox/proxy CA delivered via `SSL_CERT_FILE`.
    const TEST_CA_DER: &[u8] = include_bytes!("../tests/testdata/test-ca.der");

    fn webpki_only_len() -> usize {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        roots.len()
    }

    #[test]
    fn webpki_roots_are_always_present() {
        // The compiled-in roots keep a static binary working with no
        // system trust store and no extra CAs.
        assert!(!root_store_with([]).is_empty());
    }

    #[test]
    fn extra_ca_is_added_on_top_of_webpki_roots() {
        // Before the fix the dialer trusted webpki roots ONLY, so a proxy
        // CA like this one was never trusted and every wss:// dial failed
        // with UnknownIssuer. The merge is what makes it reachable.
        let extra = CertificateDer::from(TEST_CA_DER.to_vec());
        let merged = root_store_with([extra]);
        assert_eq!(merged.len(), webpki_only_len() + 1);
    }

    #[test]
    fn client_config_builds() {
        // Exercises the ring provider + protocol-version wiring so a
        // misconfiguration surfaces here rather than on the first dial.
        let _ = client_config();
    }
}
