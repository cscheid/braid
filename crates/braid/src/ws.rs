//! WebSocket ↔ samod transport glue.
//!
//! samod's own `tungstenite` cargo feature hardcodes `native-tls`, which
//! on Linux drags in OpenSSL and breaks static musl release builds
//! (strand br-f3b18xoa). braid therefore talks to `tokio-tungstenite`
//! directly and hands samod a [`Transport`] built here. The same
//! conversion serves both directions: the dialer in [`crate::sync`] and
//! the in-process accept loop in the sync e2e tests.

use futures::future::BoxFuture;
use futures::{SinkExt, StreamExt, TryStreamExt};
use samod::{Dialer, Transport, Url};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

/// A [`Dialer`] for `ws://` and `wss://` endpoints, with TLS provided by
/// rustls (webpki roots compiled in, so static binaries work in
/// containers that lack a system certificate store).
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
            let (ws, _response) = tokio_tungstenite::connect_async(url.as_str()).await?;
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
