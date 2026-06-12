//! Tests for viewer ui_config helpers.

use std::path::PathBuf;

use braid_config::config::DEFAULT_SYNC_SERVER;
use braid_config::ui_config::{UiConfigError, doc_url_str, ui_config};

fn make_project(tmp: &tempfile::TempDir, toml_content: &str) -> PathBuf {
    let folder = tmp.path().join("project");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(".braid.toml"), toml_content).unwrap();
    folder
}

// ---------------------------------------------------------------------------
// doc_url_str
// ---------------------------------------------------------------------------

#[test]
fn doc_url_str_adds_prefix_to_bare_id() {
    let url = doc_url_str("2An1tG1Fgqj43ViBvaD6UWfVrKZ6");
    assert_eq!(url, "automerge:2An1tG1Fgqj43ViBvaD6UWfVrKZ6");
}

#[test]
fn doc_url_str_preserves_existing_prefix() {
    let url = doc_url_str("automerge:2An1tG1Fgqj43ViBvaD6UWfVrKZ6");
    assert_eq!(url, "automerge:2An1tG1Fgqj43ViBvaD6UWfVrKZ6");
}

// ---------------------------------------------------------------------------
// ui_config: reads folder's .braid.toml directly (no walk-up, no env)
// ---------------------------------------------------------------------------

#[test]
fn ui_config_reads_folder_braid_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let folder =
        make_project(&tmp, "doc_id = \"abc123\"\nsync_server = \"wss://custom.example\"\n");

    let cfg = ui_config(&folder).unwrap();
    assert_eq!(cfg.doc_url, "automerge:abc123");
    assert_eq!(cfg.sync_server, "wss://custom.example");
}

#[test]
fn ui_config_uses_default_sync_server_when_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = make_project(&tmp, "doc_id = \"xyz789\"\n");

    let cfg = ui_config(&folder).unwrap();
    assert_eq!(cfg.doc_url, "automerge:xyz789");
    assert_eq!(cfg.sync_server, DEFAULT_SYNC_SERVER);
}

#[test]
fn ui_config_does_not_walk_up() {
    // Place .braid.toml in a parent, not the folder itself — should NOT find it.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(".braid.toml"), "doc_id = \"parent-doc\"\n").unwrap();
    let sub = tmp.path().join("sub");
    std::fs::create_dir_all(&sub).unwrap();

    let err = ui_config(&sub).unwrap_err();
    assert!(
        matches!(err, UiConfigError::NoBraidToml { .. }),
        "ui_config must not walk up; expected NoBraidToml, got {err:?}"
    );
}

#[test]
fn ui_config_ignores_env_vars() {
    // Even if BRAID_DOC_ID is set, ui_config must read from the folder only.
    let tmp = tempfile::tempdir().unwrap();
    let folder = make_project(&tmp, "doc_id = \"folder-doc\"\n");

    // Set env var that would override in the normal gather() flow
    // SAFETY: single-threaded test process, no concurrent env access
    unsafe { std::env::set_var("BRAID_DOC_ID", "env-doc-should-be-ignored") };
    let cfg = ui_config(&folder).unwrap();
    unsafe { std::env::remove_var("BRAID_DOC_ID") };

    assert_eq!(cfg.doc_url, "automerge:folder-doc");
}

#[test]
fn ui_config_errors_when_no_braid_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("empty");
    std::fs::create_dir_all(&folder).unwrap();

    let err = ui_config(&folder).unwrap_err();
    assert!(matches!(err, UiConfigError::NoBraidToml { .. }), "got {err:?}");
}

#[test]
fn ui_config_errors_when_no_doc_id() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = make_project(&tmp, "sync_server = \"wss://example.com\"\n");

    let err = ui_config(&folder).unwrap_err();
    assert!(matches!(err, UiConfigError::MissingDocId { .. }), "got {err:?}");
}
