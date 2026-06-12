//! Viewer project registry: a list of project folders persisted in
//! `viewer.toml` inside the braid config directory (paths only — never secrets).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{FileConfig, REPO_FILE_NAME};

#[derive(Debug, thiserror::Error)]
pub enum ViewerError {
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

#[derive(Debug, Default, Serialize, Deserialize)]
struct ViewerToml {
    #[serde(default)]
    projects: Vec<PathBuf>,
    #[serde(default)]
    allowed_sync_servers: Vec<String>,
}

/// Path to `viewer.toml` inside `config_dir` (`~/.config/braid/`).
pub fn viewer_toml_path(config_dir: &Path) -> PathBuf {
    config_dir.join("braid").join("viewer.toml")
}

fn load_registry(config_dir: &Path) -> Result<ViewerToml, ViewerError> {
    let path = viewer_toml_path(config_dir);
    if !path.exists() {
        return Ok(ViewerToml::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|source| ViewerError::Io { path: path.clone(), source })?;
    toml::from_str(&content).map_err(|source| ViewerError::Parse { path, source: Box::new(source) })
}

fn save_registry(config_dir: &Path, registry: &ViewerToml) -> Result<(), ViewerError> {
    let path = viewer_toml_path(config_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|source| ViewerError::Io { path: parent.to_path_buf(), source })?;
    }
    let content = toml::to_string_pretty(registry).expect("ViewerToml always serializes");
    std::fs::write(&path, content).map_err(|source| ViewerError::Io { path, source })
}

fn validate_project_folder(folder: &Path) -> Result<(), ViewerError> {
    let toml_path = folder.join(REPO_FILE_NAME);
    if !toml_path.exists() {
        return Err(ViewerError::NoBraidToml { folder: folder.to_path_buf() });
    }
    let content = std::fs::read_to_string(&toml_path)
        .map_err(|source| ViewerError::Io { path: toml_path.clone(), source })?;
    let cfg: FileConfig = toml::from_str(&content)
        .map_err(|source| ViewerError::Parse { path: toml_path, source: Box::new(source) })?;
    if cfg.doc_id.is_none() {
        return Err(ViewerError::MissingDocId { folder: folder.to_path_buf() });
    }
    Ok(())
}

/// Register a project folder. Validates that `<folder>/.braid.toml` exists
/// and contains a `doc_id`. Idempotent (duplicate add is a no-op).
pub fn add_project(folder: &Path, config_dir: &Path) -> Result<(), ViewerError> {
    validate_project_folder(folder)?;
    let mut registry = load_registry(config_dir)?;
    let canonical = folder.to_path_buf();
    if !registry.projects.contains(&canonical) {
        registry.projects.push(canonical);
        save_registry(config_dir, &registry)?;
    }
    Ok(())
}

/// Return the list of registered project folders.
pub fn list_projects(config_dir: &Path) -> Result<Vec<PathBuf>, ViewerError> {
    Ok(load_registry(config_dir)?.projects)
}

/// Remove a project folder from the registry. Idempotent.
pub fn remove_project(folder: &Path, config_dir: &Path) -> Result<(), ViewerError> {
    let mut registry = load_registry(config_dir)?;
    let before = registry.projects.len();
    registry.projects.retain(|p| p != folder);
    if registry.projects.len() != before {
        save_registry(config_dir, &registry)?;
    }
    Ok(())
}

/// Extra sync servers explicitly allowed in the CSP allowlist.
pub fn allowed_sync_servers(config_dir: &Path) -> Result<Vec<String>, ViewerError> {
    Ok(load_registry(config_dir)?.allowed_sync_servers)
}
