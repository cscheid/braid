//! Tests for layered secret/config discovery (design decisions D4, D12).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use braid::config::*;

fn file_cfg(doc_id: Option<&str>, sync: Option<&str>, author: Option<&str>) -> FileConfig {
    FileConfig {
        doc_id: doc_id.map(String::from),
        sync_server: sync.map(String::from),
        author: author.map(String::from),
    }
}

fn user_cfg(entries: &[(&str, FileConfig)]) -> UserConfig {
    UserConfig {
        projects: entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect::<BTreeMap<_, _>>(),
    }
}

/// Canonical user-config path used in resolve() tables.
const USER_CFG_PATH: &str = "/home/u/.config/braid/projects.toml";

/// A `(path, UserConfig)` pair as [`ConfigInputs::user_config`] now expects.
fn user_at(entries: &[(&str, FileConfig)]) -> (PathBuf, UserConfig) {
    (PathBuf::from(USER_CFG_PATH), user_cfg(entries))
}

/// The `Source::UserConfig` a winning user-config field produces in these
/// tables (project `proj`, canonical path).
fn user_source(project: &str) -> Source {
    Source::UserConfig { project: project.into(), path: PathBuf::from(USER_CFG_PATH) }
}

// ---------------------------------------------------------------------------
// resolve(): pure layering tables
// ---------------------------------------------------------------------------

#[test]
fn env_wins_over_everything() {
    let inputs = ConfigInputs {
        env_doc_id: Some("env-doc".into()),
        env_sync_url: Some("wss://env.example".into()),
        env_author: Some("env-author".into()),
        repo_file: Some((
            PathBuf::from("/repo/.braid.toml"),
            file_cfg(Some("file-doc"), Some("wss://file.example"), Some("file-author")),
        )),
        project_marker: Some((PathBuf::from("/repo/.braid-project"), "proj".into())),
        user_config: Some(user_at(&[(
            "proj",
            file_cfg(Some("user-doc"), Some("wss://user.example"), Some("user-author")),
        )])),
        git_user_name: Some("git-author".into()),
        os_user: Some("os-author".into()),
    };
    let got = resolve(&inputs).unwrap();
    assert_eq!(got.doc_id.expose_secret(), "env-doc");
    assert_eq!(got.sync_server, "wss://env.example");
    assert_eq!(got.author, "env-author");
    // each field reports the env var it came from
    assert_eq!(got.doc_id_source, Source::Env("BRAID_DOC_ID".into()));
    assert_eq!(got.sync_server_source, Source::Env("BRAID_SYNC_URL".into()));
    assert_eq!(got.author_source, Source::Env("BRAID_AUTHOR".into()));
}

#[test]
fn repo_file_wins_over_user_config() {
    let inputs = ConfigInputs {
        repo_file: Some((
            PathBuf::from("/repo/.braid.toml"),
            file_cfg(Some("file-doc"), None, None),
        )),
        project_marker: Some((PathBuf::from("/repo/.braid-project"), "proj".into())),
        user_config: Some(user_at(&[(
            "proj",
            file_cfg(Some("user-doc"), Some("wss://user.example"), Some("user-author")),
        )])),
        git_user_name: Some("git-author".into()),
        os_user: Some("os-author".into()),
        ..Default::default()
    };
    let got = resolve(&inputs).unwrap();
    assert_eq!(got.doc_id.expose_secret(), "file-doc");
    assert_eq!(got.doc_id_source, Source::RepoFile("/repo/.braid.toml".into()));
    // per-field independence: sync_server and author keep falling through to
    // the user config, and each reports that distinct source
    assert_eq!(got.sync_server, "wss://user.example");
    assert_eq!(got.author, "user-author");
    assert_eq!(got.sync_server_source, user_source("proj"));
    assert_eq!(got.author_source, user_source("proj"));
}

#[test]
fn user_config_via_marker() {
    let inputs = ConfigInputs {
        project_marker: Some((PathBuf::from("/repo/.braid-project"), "proj".into())),
        user_config: Some(user_at(&[("proj", file_cfg(Some("user-doc"), None, None))])),
        git_user_name: Some("git-author".into()),
        os_user: Some("os-author".into()),
        ..Default::default()
    };
    let got = resolve(&inputs).unwrap();
    assert_eq!(got.doc_id.expose_secret(), "user-doc");
    assert_eq!(got.doc_id_source, user_source("proj"));
    assert_eq!(got.sync_server, DEFAULT_SYNC_SERVER, "default server fallback");
    assert_eq!(got.sync_server_source, Source::Default);
    assert_eq!(got.author, "git-author", "author falls through to git");
    assert_eq!(got.author_source, Source::GitConfig);
}

#[test]
fn author_chain_falls_back_to_os_user_then_unknown() {
    let base = ConfigInputs { env_doc_id: Some("env-doc".into()), ..Default::default() };

    let inputs = ConfigInputs { os_user: Some("os-author".into()), ..base.clone() };
    let got = resolve(&inputs).unwrap();
    assert_eq!(got.author, "os-author");
    assert_eq!(got.author_source, Source::OsUser);

    let got = resolve(&base).unwrap();
    assert_eq!(got.author, "unknown");
    assert_eq!(got.author_source, Source::Default);
}

#[test]
fn no_doc_id_anywhere_is_a_helpful_error() {
    let inputs = ConfigInputs {
        git_user_name: Some("git-author".into()),
        os_user: Some("os".into()),
        ..Default::default()
    };
    let err = resolve(&inputs).unwrap_err();
    assert!(matches!(err, ConfigError::NoDocId));
    let msg = err.to_string();
    for needle in ["BRAID_DOC_ID", ".braid.toml", ".braid-project", "braid init"] {
        assert!(msg.contains(needle), "error message should mention {needle}: {msg}");
    }
}

#[test]
fn marker_naming_missing_project_is_an_error() {
    let inputs = ConfigInputs {
        project_marker: Some((PathBuf::from("/repo/.braid-project"), "ghost".into())),
        user_config: Some(user_at(&[("proj", file_cfg(Some("user-doc"), None, None))])),
        ..Default::default()
    };
    let err = resolve(&inputs).unwrap_err();
    match err {
        ConfigError::UnknownProject { project, .. } => assert_eq!(project, "ghost"),
        other => panic!("expected UnknownProject, got {other:?}"),
    }
}

#[test]
fn marker_without_any_user_config_is_unknown_project() {
    let inputs = ConfigInputs {
        project_marker: Some((PathBuf::from("/repo/.braid-project"), "proj".into())),
        user_config: None,
        ..Default::default()
    };
    assert!(matches!(resolve(&inputs).unwrap_err(), ConfigError::UnknownProject { .. }));
}

#[test]
fn repo_file_without_doc_id_still_contributes_other_fields() {
    // .braid.toml that only pins the sync server; doc id comes from env.
    let inputs = ConfigInputs {
        env_doc_id: Some("env-doc".into()),
        repo_file: Some((
            PathBuf::from("/repo/.braid.toml"),
            file_cfg(None, Some("wss://file.example"), None),
        )),
        ..Default::default()
    };
    let got = resolve(&inputs).unwrap();
    assert_eq!(got.doc_id.expose_secret(), "env-doc");
    assert_eq!(got.doc_id_source, Source::Env("BRAID_DOC_ID".into()));
    assert_eq!(got.sync_server, "wss://file.example");
    assert_eq!(got.sync_server_source, Source::RepoFile("/repo/.braid.toml".into()));
}

// ---------------------------------------------------------------------------
// Source::describe(): human-readable provenance for `braid config`/`secret`
// ---------------------------------------------------------------------------

#[test]
fn source_describe_names_each_origin() {
    assert_eq!(Source::Env("BRAID_DOC_ID".into()).describe(), "BRAID_DOC_ID environment variable");
    assert_eq!(Source::RepoFile("/repo/.braid.toml".into()).describe(), "/repo/.braid.toml");
    assert_eq!(
        Source::UserConfig { project: "q2".into(), path: PathBuf::from(USER_CFG_PATH) }.describe(),
        format!("{USER_CFG_PATH} [projects.q2]"),
    );
    assert_eq!(Source::GitConfig.describe(), "git config user.name");
    // OS-username and built-in-default descriptions are stable, human prose
    assert!(Source::OsUser.describe().contains("USER"));
    assert!(Source::Default.describe().contains("default"));
}

// ---------------------------------------------------------------------------
// gather_fs(): filesystem discovery
// ---------------------------------------------------------------------------

fn env_map(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: HashMap<String, String> =
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |k: &str| map.get(k).cloned()
}

#[test]
fn gather_walks_up_for_repo_file_and_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join(".braid.toml"), "doc_id = \"from-file\"\n").unwrap();
    std::fs::write(root.join(".braid-project"), "myproj\n").unwrap();
    let deep = root.join("a/b/c");
    std::fs::create_dir_all(&deep).unwrap();

    let inputs = gather_fs(&deep, &env_map(&[])).unwrap();
    let (path, cfg) = inputs.repo_file.expect("should find .braid.toml in ancestor");
    assert_eq!(path, root.join(".braid.toml"));
    assert_eq!(cfg.doc_id.as_deref(), Some("from-file"));
    let (mpath, name) = inputs.project_marker.expect("should find marker in ancestor");
    assert_eq!(mpath, root.join(".braid-project"));
    assert_eq!(name, "myproj");
}

#[test]
fn gather_nearest_file_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join(".braid.toml"), "doc_id = \"outer\"\n").unwrap();
    let deep = root.join("inner");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join(".braid.toml"), "doc_id = \"inner\"\n").unwrap();

    let inputs = gather_fs(&deep, &env_map(&[])).unwrap();
    assert_eq!(inputs.repo_file.unwrap().1.doc_id.as_deref(), Some("inner"));
}

#[test]
fn gather_reads_user_config_via_xdg() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_home = tmp.path().join("xdg-config");
    std::fs::create_dir_all(cfg_home.join("braid")).unwrap();
    std::fs::write(
        cfg_home.join("braid/projects.toml"),
        "[projects.myproj]\ndoc_id = \"from-user-config\"\n",
    )
    .unwrap();
    let cwd = tmp.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();

    let env = env_map(&[("XDG_CONFIG_HOME", cfg_home.to_str().unwrap())]);
    let inputs = gather_fs(&cwd, &env).unwrap();
    let (path, uc) = inputs.user_config.expect("user config should load");
    assert_eq!(path, cfg_home.join("braid/projects.toml"), "remembers the resolved path");
    assert_eq!(uc.projects["myproj"].doc_id.as_deref(), Some("from-user-config"));
}

#[test]
fn user_config_path_falls_back_to_userprofile_on_windows() {
    // XDG wins when set
    let e = env_map(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/home/u")]);
    assert_eq!(user_config_path(&e), Some(PathBuf::from("/xdg/braid/projects.toml")));

    // HOME otherwise
    let e = env_map(&[("HOME", "/home/u")]);
    assert_eq!(user_config_path(&e), Some(PathBuf::from("/home/u/.config/braid/projects.toml")));

    // Windows: no HOME → USERPROFILE
    let e = env_map(&[("USERPROFILE", "/users/u")]);
    assert_eq!(user_config_path(&e), Some(PathBuf::from("/users/u/.config/braid/projects.toml")));

    // HOME wins over USERPROFILE
    let e = env_map(&[("HOME", "/home/u"), ("USERPROFILE", "/users/u")]);
    assert_eq!(user_config_path(&e), Some(PathBuf::from("/home/u/.config/braid/projects.toml")));

    // neither → None
    let e = env_map(&[]);
    assert_eq!(user_config_path(&e), None);
}

#[test]
fn gather_picks_up_env_vars() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_map(&[
        ("BRAID_DOC_ID", "env-doc"),
        ("BRAID_SYNC_URL", "wss://env.example"),
        ("BRAID_AUTHOR", "env-author"),
    ]);
    let inputs = gather_fs(tmp.path(), &env).unwrap();
    assert_eq!(inputs.env_doc_id.as_deref(), Some("env-doc"));
    assert_eq!(inputs.env_sync_url.as_deref(), Some("wss://env.example"));
    assert_eq!(inputs.env_author.as_deref(), Some("env-author"));
}

#[test]
fn gather_treats_empty_env_vars_as_unset() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_map(&[("BRAID_DOC_ID", ""), ("BRAID_AUTHOR", "  ")]);
    let inputs = gather_fs(tmp.path(), &env).unwrap();
    assert_eq!(inputs.env_doc_id, None);
    assert_eq!(inputs.env_author, None);
}

#[test]
fn gather_propagates_parse_errors_with_path() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(".braid.toml"), "this is not toml [[[").unwrap();
    let err = gather_fs(tmp.path(), &env_map(&[])).unwrap_err();
    match err {
        ConfigError::Parse { path, .. } => assert_eq!(path, tmp.path().join(".braid.toml")),
        other => panic!("expected Parse error, got {other:?}"),
    }
}

#[test]
fn gather_rejects_unknown_keys_in_braid_toml() {
    // Catches typos like `docid = ...` instead of silently ignoring them.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(".braid.toml"), "docid = \"oops\"\n").unwrap();
    assert!(matches!(gather_fs(tmp.path(), &env_map(&[])).unwrap_err(), ConfigError::Parse { .. }));
}

#[test]
fn missing_everything_gathers_cleanly_then_fails_resolution() {
    let tmp = tempfile::tempdir().unwrap();
    let inputs = gather_fs(tmp.path(), &env_map(&[])).unwrap();
    assert!(matches!(resolve(&inputs).unwrap_err(), ConfigError::NoDocId));
}
