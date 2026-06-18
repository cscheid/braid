//! braid-viewer Tauri backend — thin shell over braid-config.
//!
//! Commands expose project registry operations and per-project config to the
//! webview. The webview syncs **directly** to the automerge sync server; this
//! process only supplies config and opens the native folder picker.
//!
//! Never log UiConfig / docUrl — the doc id is a bearer secret.

use std::path::PathBuf;

use braid_config::ui_config::{ui_config, UiConfig};
use braid_config::viewer::{allowed_sync_servers, list_projects};
use tauri::Manager;
use tauri_plugin_log::{Target, TargetKind};

pub mod csp;

/// The XDG config dir for the viewer registry, injected at setup time.
pub struct ConfigDir(pub PathBuf);

// ---------------------------------------------------------------------------
// Commands in a submodule to avoid E0255 in Rust ≥1.87.
//
// `#[tauri::command]` on a `pub fn` emits both `#[macro_export]` (placing the
// helper macro at the crate root) and `pub use {macro}` (reimporting it into
// the current module).  When those two operations are in the crate root itself
// (lib.rs) the define + import cycle is an error.  A submodule breaks the
// cycle: the `use` import lands in `mod commands`, not in the crate root.
pub mod commands {
    use std::path::PathBuf;

    use braid_config::ui_config::{ui_config, UiConfigError};
    use braid_config::viewer::{add_project, list_projects, remove_project, ViewerError};
    use serde::Serialize;
    use tauri::State;

    use super::ConfigDir;

    // ---------------------------------------------------------------------------
    // Error type surfaced to the frontend

    #[derive(Debug, thiserror::Error, Serialize)]
    #[serde(tag = "kind", content = "message")]
    pub enum ViewerCommandError {
        #[error("viewer registry error: {0}")]
        Registry(String),
        #[error("config error: {0}")]
        Config(String),
    }

    impl From<ViewerError> for ViewerCommandError {
        fn from(e: ViewerError) -> Self {
            Self::Registry(e.to_string())
        }
    }

    impl From<UiConfigError> for ViewerCommandError {
        fn from(e: UiConfigError) -> Self {
            Self::Config(e.to_string())
        }
    }

    /// The UI-safe config payload sent to the webview.
    #[derive(Serialize)]
    pub struct UiConfigPayload {
        pub doc_url: String,
        pub sync_server: String,
    }

    /// List registered project folder paths.
    #[tauri::command]
    pub fn list_projects_cmd(
        config_dir: State<'_, ConfigDir>,
    ) -> Result<Vec<PathBuf>, ViewerCommandError> {
        Ok(list_projects(&config_dir.0)?)
    }

    /// Register a new project folder. Validates `<folder>/.braid.toml` exists
    /// with a `doc_id` before adding. Idempotent.
    #[tauri::command]
    pub fn add_project_cmd(
        folder: PathBuf,
        config_dir: State<'_, ConfigDir>,
    ) -> Result<(), ViewerCommandError> {
        Ok(add_project(&folder, &config_dir.0)?)
    }

    /// Remove a project folder from the registry. Idempotent.
    #[tauri::command]
    pub fn remove_project_cmd(
        folder: PathBuf,
        config_dir: State<'_, ConfigDir>,
    ) -> Result<(), ViewerCommandError> {
        Ok(remove_project(&folder, &config_dir.0)?)
    }

    /// Return the UI config (docUrl + syncServer) for a registered project folder.
    /// Never returns the raw doc id — only the `automerge:` prefixed URL.
    #[tauri::command]
    pub fn get_config_cmd(folder: PathBuf) -> Result<UiConfigPayload, ViewerCommandError> {
        let cfg = ui_config(&folder)?;
        Ok(UiConfigPayload { doc_url: cfg.doc_url, sync_server: cfg.sync_server })
    }
}

// ---------------------------------------------------------------------------
// App setup

/// Build the Tauri application.
///
/// Computes the CSP allowlist from registered projects' sync servers plus the
/// `allowed_sync_servers` list in viewer.toml, then starts Tauri.
pub fn run() {
    tauri::Builder::default()
        // Logging first so the rest of setup (and any plugin) can emit records.
        // Writes to stdout, a rotating file in the platform log dir
        // (`braid-viewer.log` under e.g. %APPDATA%/<id>/logs on Windows,
        // ~/.local/share/<id>/logs on Linux, ~/Library/Logs/<id> on macOS), and
        // forwards to the webview console. This is what gives the *release* exe
        // a trace at all: it runs with `windows_subsystem = "windows"` (no
        // console) and would otherwise fail completely silently.
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                    Target::new(TargetKind::Webview),
                ])
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            log::info!("braid-viewer v{} starting", env!("CARGO_PKG_VERSION"));

            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| PathBuf::from(".config/braid-viewer"));
            // Pin the webview data directory so IndexedDB — and therefore
            // offline reads / warm starts — persist deterministically across
            // restarts. Tauri v2 exposes `data_directory` only on the
            // per-window builder, so the main window is created here instead of
            // declared in tauri.conf.json.
            let data_dir =
                app.path().app_local_data_dir().unwrap_or_else(|_| config_dir.join("data"));
            app.manage(ConfigDir(config_dir.clone()));

            // Sync servers to allow in the webview CSP beyond the default one
            // baked into tauri.conf.json: every registered project's server plus
            // any explicitly listed in viewer.toml. Best-effort — a bad
            // .braid.toml must not crash startup. This is a startup snapshot: a
            // project added at runtime on a non-default server needs a restart
            // (or pre-declaration in `allowed_sync_servers`) before it can sync.
            let mut extra_servers: Vec<String> = list_projects(&config_dir)
                .unwrap_or_default()
                .iter()
                .filter_map(|folder| ui_config(folder).ok())
                .map(|c: UiConfig| c.sync_server)
                .chain(allowed_sync_servers(&config_dir).unwrap_or_default())
                .filter(|s| s != "wss://sync.automerge.org")
                .collect();
            extra_servers.sort();
            extra_servers.dedup();

            tauri::WebviewWindowBuilder::new(app.handle(), "main", tauri::WebviewUrl::default())
                .title("braid viewer")
                .inner_size(1200.0, 800.0)
                .min_inner_size(800.0, 600.0)
                .data_directory(data_dir.clone())
                // Append the extra sync servers to the static CSP's `connect-src`
                // so non-default servers aren't blocked in the packaged app. Only
                // fires for the `tauri://` protocol (the packaged frontend); the
                // dev server is untouched and needs no CSP.
                .on_web_resource_request(move |request, response| {
                    if extra_servers.is_empty() || request.uri().scheme_str() != Some("tauri") {
                        return;
                    }
                    if let Some(header) = response.headers_mut().get_mut("Content-Security-Policy")
                    {
                        if let Ok(current) = header.to_str() {
                            let augmented = csp::augment_connect_src(current, &extra_servers);
                            if let Ok(value) = tauri::http::HeaderValue::from_str(&augmented) {
                                *header = value;
                            }
                        }
                    }
                })
                .build()?;
            log::info!("webview data dir: {}", data_dir.display());

            // Diagnostic: log each window's resolved URL. A binary built via the
            // Tauri CLI (`cargo tauri build`/`xtask viewer-build`) embeds the
            // frontend and shows a `tauri://localhost` / `http://tauri.localhost`
            // URL. A binary built with a plain `cargo build --release` omits the
            // `custom-protocol` feature, runs in dev mode, and shows the Vite dev
            // URL (`http://localhost:5173`) — which fails with
            // ERR_CONNECTION_REFUSED when no dev server is running. These URLs are
            // not secrets (unlike docUrl, which must never be logged).
            for (label, win) in app.webview_windows() {
                match win.url() {
                    Ok(url) => log::info!("window '{label}' loading {url}"),
                    Err(e) => log::warn!("window '{label}' url unavailable: {e}"),
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_projects_cmd,
            commands::add_project_cmd,
            commands::remove_project_cmd,
            commands::get_config_cmd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running braid-viewer");
}
