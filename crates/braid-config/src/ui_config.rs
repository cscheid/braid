//! UI-config helpers for the viewer: `doc_url_str` and `ui_config`.
//!
//! `ui_config(folder)` reads `<folder>/.braid.toml` **directly** — no
//! walk-up, no `BRAID_*` env — to enforce strict folder semantics for the
//! viewer's project selector.

use std::path::{Path, PathBuf};

use crate::config::{DEFAULT_SYNC_SERVER, FileConfig, REPO_FILE_NAME};

/// Config returned to the webview for a specific project folder.
#[derive(Debug)]
pub struct UiConfig {
    pub doc_url: String,
    pub sync_server: String,
}

#[derive(Debug, thiserror::Error)]
pub enum UiConfigError {
    #[error("no .braid.toml found in {folder}")]
    NoBraidToml { folder: PathBuf },
    #[error("{folder}/.braid.toml has no doc_id")]
    MissingDocId { folder: PathBuf },
    #[error("could not parse {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Build the `automerge:<id>` URL from a raw doc id string, adding the
/// prefix only if the id doesn't already have it.
pub fn doc_url_str(raw: &str) -> String {
    if raw.starts_with("automerge:") { raw.to_string() } else { format!("automerge:{raw}") }
}

/// Read `<folder>/.braid.toml` and return the UI config for the viewer.
///
/// Strict semantics: reads only `<folder>/.braid.toml`, never walks up,
/// never consults `BRAID_*` environment variables.
pub fn ui_config(folder: &Path) -> Result<UiConfig, UiConfigError> {
    let toml_path = folder.join(REPO_FILE_NAME);
    if !toml_path.exists() {
        return Err(UiConfigError::NoBraidToml { folder: folder.to_path_buf() });
    }
    let content = std::fs::read_to_string(&toml_path)
        .map_err(|source| UiConfigError::Io { path: toml_path.clone(), source })?;
    let cfg: FileConfig = toml::from_str(&content)
        .map_err(|source| UiConfigError::Parse { path: toml_path, source: Box::new(source) })?;
    let raw =
        cfg.doc_id.ok_or_else(|| UiConfigError::MissingDocId { folder: folder.to_path_buf() })?;
    Ok(UiConfig {
        doc_url: doc_url_str(&raw),
        sync_server: cfg.sync_server.unwrap_or_else(|| DEFAULT_SYNC_SERVER.to_string()),
    })
}
