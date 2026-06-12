//! `braid ui` — serve the React skein UI locally and open it in the browser.
//!
//! The local server is deliberately minimal:
//!
//! - `GET /api/config` returns `{"docUrl":"automerge:…","syncServer":"wss://…"}`.
//!   This is the **only** place the bearer secret touches the network — served
//!   only on `127.0.0.1`, never in the browser URL, never in localStorage.
//!
//! - `GET /` and everything else serves the embedded React app built from
//!   `ui/dist/` (embedded at compile time via `rust-embed`).
//!
//! The browser then connects **directly** to the automerge sync server via
//! WebSocket; no CRDT traffic proxies through this process.  Ctrl-C stops it.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use serde_json::json;
use tokio::net::TcpListener;

use crate::config;

// Embed the pre-built React app. Path is relative to this crate's Cargo.toml.
#[derive(rust_embed::Embed)]
#[folder = "../../ui/dist"]
struct Assets;

// ---------------------------------------------------------------------------

struct UiState {
    doc_url: String,
    sync_server: String,
}

pub async fn serve(cwd: &Path) -> Result<()> {
    let cfg = config::load(cwd)?;

    // expose_secret() returns whatever string was stored in .braid.toml.
    // The automerge-repo JS library expects "automerge:<id>"; add the prefix
    // if the stored value is the bare id (samod historically omitted it).
    let raw = cfg.doc_id.expose_secret();
    let doc_url =
        if raw.starts_with("automerge:") { raw.to_string() } else { format!("automerge:{raw}") };

    let state = Arc::new(UiState { doc_url, sync_server: cfg.sync_server.clone() });

    let app = Router::new()
        .route("/api/config", get(config_handler))
        .fallback(static_handler)
        .with_state(state);

    // Bind to a random loopback port so we never conflict with anything.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let url = format!("http://{addr}");

    eprintln!("braid ui  →  {url}");
    eprintln!("Press Ctrl-C to stop.");
    open_browser(&url);

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;

    eprintln!("\nbraid ui: stopped.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers

async fn config_handler(State(s): State<Arc<UiState>>) -> Json<serde_json::Value> {
    Json(json!({ "docUrl": s.doc_url, "syncServer": s.sync_server }))
}

async fn static_handler(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    serve_asset(path).unwrap_or_else(|| {
        // SPA fallback — all unknown paths get index.html so the React router
        // can handle them (currently we have no client-side routes, but this
        // makes the setup future-proof).
        serve_asset("index.html").unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
    })
}

fn serve_asset(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let mime = mime_for(path);
    Some(([(header::CONTENT_TYPE, mime)], file.data.to_vec()).into_response())
}

/// Minimal MIME mapping covering everything Vite's output directory contains.
fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------------------

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/c", "start", url]).spawn();
}
