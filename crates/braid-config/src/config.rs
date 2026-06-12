//! Layered secret/config discovery (design decisions D4, D12).
//!
//! The (sync server, document id) pair is a read/write bearer secret. It is
//! resolved per-field, first hit wins:
//!
//! 1. environment: `BRAID_DOC_ID`, `BRAID_SYNC_URL`, `BRAID_AUTHOR`
//! 2. a gitignored `.braid.toml` found by walking up from the cwd
//! 3. `$XDG_CONFIG_HOME/braid/projects.toml` (default `~/.config/braid/`),
//!    selected by a committed, non-secret `.braid-project` marker file
//!    (also found by walk-up) containing a project name
//!
//! `sync_server` additionally falls back to [`DEFAULT_SYNC_SERVER`]; the
//! author chain continues through `git config user.name` and `$USER`.
//!
//! Structure: [`resolve`] is pure (table-testable); [`gather_fs`] performs
//! filesystem discovery with an injectable environment lookup;
//! [`gather`] composes everything including the git/OS probes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::docid::DocId;

pub const DEFAULT_SYNC_SERVER: &str = "wss://sync.automerge.org";
pub const REPO_FILE_NAME: &str = ".braid.toml";
pub const PROJECT_MARKER_NAME: &str = ".braid-project";

/// Shape of `.braid.toml` and of each `[projects.<name>]` table in the
/// user-level config.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub doc_id: Option<String>,
    pub sync_server: Option<String>,
    pub author: Option<String>,
}

/// Shape of `~/.config/braid/projects.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    #[serde(default)]
    pub projects: BTreeMap<String, FileConfig>,
}

/// Everything resolution needs, gathered from the environment beforehand so
/// that [`resolve`] itself is pure.
#[derive(Debug, Clone, Default)]
pub struct ConfigInputs {
    pub env_doc_id: Option<String>,
    pub env_sync_url: Option<String>,
    pub env_author: Option<String>,
    /// Nearest `.braid.toml` walking up from cwd, with its path.
    pub repo_file: Option<(PathBuf, FileConfig)>,
    /// Nearest `.braid-project` marker walking up from cwd: (path, name).
    pub project_marker: Option<(PathBuf, String)>,
    /// User-level config with the path it was read from (honoring XDG), so
    /// diagnostics can point at the exact file.
    pub user_config: Option<(PathBuf, UserConfig)>,
    pub git_user_name: Option<String>,
    pub os_user: Option<String>,
}

/// Where a resolved field came from, for diagnostics (`braid config`,
/// `braid secret`). `doc_id` can only ever be `Env`/`RepoFile`/`UserConfig`;
/// `sync_server` adds `Default`; `author` adds `GitConfig`/`OsUser`/`Default`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// An environment variable, named (e.g. `BRAID_DOC_ID`).
    Env(String),
    /// A `.braid.toml` at this path.
    RepoFile(PathBuf),
    /// The `[projects.<project>]` table in the user config at this path.
    UserConfig { project: String, path: PathBuf },
    /// `git config user.name` (author only).
    GitConfig,
    /// The OS username, `$USER` / `$USERNAME` (author only).
    OsUser,
    /// A built-in default: the sync server, or the `"unknown"` author.
    Default,
}

impl Source {
    /// Human-readable provenance for CLI output.
    pub fn describe(&self) -> String {
        match self {
            Source::Env(var) => format!("{var} environment variable"),
            Source::RepoFile(path) => path.display().to_string(),
            Source::UserConfig { project, path } => {
                format!("{} [projects.{project}]", path.display())
            }
            Source::GitConfig => "git config user.name".to_string(),
            Source::OsUser => "OS username ($USER / $USERNAME)".to_string(),
            Source::Default => "built-in default".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub doc_id: DocId,
    pub sync_server: String,
    pub author: String,
    pub doc_id_source: Source,
    pub sync_server_source: Source,
    pub author_source: Source,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "no skein configured: could not find a document id.\n\
         braid looks for one in (first hit wins):\n\
         1. the BRAID_DOC_ID environment variable\n\
         2. a .braid.toml file in this directory or any parent\n\
         3. ~/.config/braid/projects.toml, selected by a .braid-project marker file\n\
         Run `braid init` to create a new skein here, or\n\
         `braid init --join <doc-id>` to adopt an existing one."
    )]
    NoDocId,
    #[error(
        "project marker {marker} names project {project:?}, but \
         {user_config} has no [projects.{project}] entry with a doc_id"
    )]
    UnknownProject { project: String, marker: PathBuf, user_config: String },
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse {path}: {source}")]
    Parse {
        path: PathBuf,
        // Boxed: `toml::de::Error` is large enough to push `ConfigError`
        // (and every `Result<_, ConfigError>`) over clippy's
        // result_large_err threshold on some targets (Windows). Boxing
        // keeps the error — and the common Ok path — small.
        #[source]
        source: Box<toml::de::Error>,
    },
}

/// Pure, per-field layered resolution. See module docs for the layering.
pub fn resolve(inputs: &ConfigInputs) -> Result<ResolvedConfig, ConfigError> {
    // The user-config layer applies only when a marker names a project. Carry
    // the config's path so winning user-config fields can name their source.
    let project_cfg: Option<(&Path, &str, &FileConfig)> = match &inputs.project_marker {
        Some((_, name)) => inputs
            .user_config
            .as_ref()
            .and_then(|(path, uc)| uc.projects.get(name).map(|cfg| (path.as_path(), name, cfg)))
            .map(|(path, name, cfg)| (path, name.as_str(), cfg)),
        None => None,
    };
    let user_source = |project: &str, path: &Path| Source::UserConfig {
        project: project.to_string(),
        path: path.to_path_buf(),
    };

    // doc_id: env > repo file > user config (via marker).
    let (doc_id, doc_id_source) = if let Some(id) = &inputs.env_doc_id {
        (id.clone(), Source::Env("BRAID_DOC_ID".to_string()))
    } else if let Some((path, FileConfig { doc_id: Some(id), .. })) = &inputs.repo_file {
        (id.clone(), Source::RepoFile(path.clone()))
    } else if let Some((path, project, FileConfig { doc_id: Some(id), .. })) = project_cfg {
        (id.clone(), user_source(project, path))
    } else if let Some((marker, project)) = &inputs.project_marker {
        // A marker promised a project, but the user config doesn't deliver
        // a doc_id for it — that deserves a more specific error than the
        // generic "nothing configured".
        return Err(ConfigError::UnknownProject {
            project: project.clone(),
            marker: marker.clone(),
            user_config: "~/.config/braid/projects.toml".to_string(),
        });
    } else {
        return Err(ConfigError::NoDocId);
    };

    // sync_server: env > repo file > user config > built-in default.
    let (sync_server, sync_server_source) = if let Some(v) = &inputs.env_sync_url {
        (v.clone(), Source::Env("BRAID_SYNC_URL".to_string()))
    } else if let Some((path, FileConfig { sync_server: Some(v), .. })) = &inputs.repo_file {
        (v.clone(), Source::RepoFile(path.clone()))
    } else if let Some((path, project, FileConfig { sync_server: Some(v), .. })) = project_cfg {
        (v.clone(), user_source(project, path))
    } else {
        (DEFAULT_SYNC_SERVER.to_string(), Source::Default)
    };

    // author: env > repo file > user config > git config > OS user > unknown.
    let (author, author_source) = if let Some(v) = &inputs.env_author {
        (v.clone(), Source::Env("BRAID_AUTHOR".to_string()))
    } else if let Some((path, FileConfig { author: Some(v), .. })) = &inputs.repo_file {
        (v.clone(), Source::RepoFile(path.clone()))
    } else if let Some((path, project, FileConfig { author: Some(v), .. })) = project_cfg {
        (v.clone(), user_source(project, path))
    } else if let Some(v) = &inputs.git_user_name {
        (v.clone(), Source::GitConfig)
    } else if let Some(v) = &inputs.os_user {
        (v.clone(), Source::OsUser)
    } else {
        ("unknown".to_string(), Source::Default)
    };

    Ok(ResolvedConfig {
        doc_id: DocId::new(doc_id),
        sync_server,
        author,
        doc_id_source,
        sync_server_source,
        author_source,
    })
}

fn non_blank(v: Option<String>) -> Option<String> {
    v.and_then(|s| {
        let t = s.trim();
        if t.is_empty() { None } else { Some(t.to_string()) }
    })
}

/// Walk from `start` to the filesystem root, returning the first ancestor
/// (including `start`) containing `name`.
fn find_up(start: &Path, name: &str) -> Option<PathBuf> {
    start.ancestors().map(|d| d.join(name)).find(|p| p.is_file())
}

fn read_to_string(path: &Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path)
        .map_err(|source| ConfigError::Io { path: path.to_path_buf(), source })
}

fn parse_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, ConfigError> {
    toml::from_str(&read_to_string(path)?)
        .map_err(|source| ConfigError::Parse { path: path.to_path_buf(), source: Box::new(source) })
}

/// Filesystem + environment discovery, with an injectable env lookup (for
/// tests). Leaves `git_user_name` / `os_user` as `None`; [`gather`] fills
/// those in.
pub fn gather_fs(
    cwd: &Path,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<ConfigInputs, ConfigError> {
    let repo_file = match find_up(cwd, REPO_FILE_NAME) {
        Some(path) => {
            let cfg: FileConfig = parse_toml(&path)?;
            Some((path, cfg))
        }
        None => None,
    };

    let project_marker = match find_up(cwd, PROJECT_MARKER_NAME) {
        Some(path) => {
            let name = read_to_string(&path)?.trim().to_string();
            if name.is_empty() { None } else { Some((path, name)) }
        }
        None => None,
    };

    let user_config = match user_config_path(env) {
        Some(path) if path.is_file() => {
            let cfg = parse_toml::<UserConfig>(&path)?;
            Some((path, cfg))
        }
        _ => None,
    };

    Ok(ConfigInputs {
        env_doc_id: non_blank(env("BRAID_DOC_ID")),
        env_sync_url: non_blank(env("BRAID_SYNC_URL")),
        env_author: non_blank(env("BRAID_AUTHOR")),
        repo_file,
        project_marker,
        user_config,
        git_user_name: None,
        os_user: None,
    })
}

/// Full discovery against the real process environment, including the
/// `git config user.name` and OS-username probes for the author chain.
pub fn gather(cwd: &Path) -> Result<ConfigInputs, ConfigError> {
    let env = |k: &str| std::env::var(k).ok();
    let mut inputs = gather_fs(cwd, &env)?;
    inputs.git_user_name = git_user_name(cwd);
    inputs.os_user = non_blank(env("USER")).or_else(|| non_blank(env("USERNAME")));
    Ok(inputs)
}

fn git_user_name(cwd: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["config", "--get", "user.name"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    non_blank(Some(String::from_utf8_lossy(&out.stdout).to_string()))
}

/// Path of the user-level config file, honoring `XDG_CONFIG_HOME`. The home
/// directory is `HOME`, falling back to `USERPROFILE` on Windows (where
/// `HOME` is usually unset).
pub fn user_config_path(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let home = || non_blank(env("HOME")).or_else(|| non_blank(env("USERPROFILE")));
    let base = match non_blank(env("XDG_CONFIG_HOME")) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(home()?).join(".config"),
    };
    Some(base.join("braid").join("projects.toml"))
}

/// Convenience: gather + resolve from cwd.
pub fn load(cwd: &Path) -> Result<ResolvedConfig, ConfigError> {
    resolve(&gather(cwd)?)
}
