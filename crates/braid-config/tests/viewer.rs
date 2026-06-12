//! Tests for the viewer project registry (viewer.toml).

use std::path::PathBuf;

use braid_config::viewer::{
    ViewerError, add_project, list_projects, remove_project, viewer_toml_path,
};

fn make_project(tmp: &tempfile::TempDir, doc_id: &str) -> PathBuf {
    let folder = tmp.path().join("projects").join(doc_id);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(".braid.toml"), format!("doc_id = \"{doc_id}\"\n")).unwrap();
    folder
}

fn viewer_toml_in(tmp: &tempfile::TempDir) -> PathBuf {
    viewer_toml_path(tmp.path())
}

// ---------------------------------------------------------------------------
// add/list/remove round-trip
// ---------------------------------------------------------------------------

#[test]
fn add_list_remove_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_path_buf();
    let proj = make_project(&tmp, "abc123");

    add_project(&proj, &config_dir).unwrap();
    let projects = list_projects(&config_dir).unwrap();
    assert!(projects.contains(&proj), "project should appear in list after add");

    remove_project(&proj, &config_dir).unwrap();
    let projects = list_projects(&config_dir).unwrap();
    assert!(!projects.contains(&proj), "project should be gone after remove");
}

#[test]
fn add_same_project_twice_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_path_buf();
    let proj = make_project(&tmp, "idm001");

    add_project(&proj, &config_dir).unwrap();
    add_project(&proj, &config_dir).unwrap();

    let projects = list_projects(&config_dir).unwrap();
    let count = projects.iter().filter(|p| *p == &proj).count();
    assert_eq!(count, 1, "duplicate add should not create duplicate entries");
}

#[test]
fn list_with_no_viewer_toml_returns_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_path_buf();
    let projects = list_projects(&config_dir).unwrap();
    assert!(projects.is_empty());
}

#[test]
fn remove_nonexistent_project_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_path_buf();
    let missing = PathBuf::from("/nonexistent/path/to/project");
    // Should not error
    remove_project(&missing, &config_dir).unwrap();
}

// ---------------------------------------------------------------------------
// Secret hygiene: viewer.toml must never contain doc_id or docUrl
// ---------------------------------------------------------------------------

#[test]
fn viewer_toml_contains_no_secret_substrings() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_path_buf();

    // Use a neutral folder name; the doc_id lives inside .braid.toml only
    let secret_doc_id = "2An1tG1Fgqj43ViBvaD6UWfVrKZ6";
    let proj = tmp.path().join("projects").join("my-skein");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join(".braid.toml"), format!("doc_id = \"{secret_doc_id}\"\n")).unwrap();

    add_project(&proj, &config_dir).unwrap();

    let toml_path = viewer_toml_in(&tmp);
    let content = std::fs::read_to_string(&toml_path).unwrap();

    // The viewer.toml must store paths only — never the doc id
    assert!(
        !content.contains(secret_doc_id),
        "viewer.toml must not contain the doc_id:\n{content}"
    );
    assert!(!content.contains("doc_id"), "viewer.toml must not contain 'doc_id':\n{content}");
    assert!(!content.contains("docUrl"), "viewer.toml must not contain 'docUrl':\n{content}");
    assert!(
        !content.contains("automerge:"),
        "viewer.toml must not contain 'automerge:' URLs:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// add_project validates the folder
// ---------------------------------------------------------------------------

#[test]
fn add_project_requires_braid_toml_with_doc_id() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_path_buf();

    // Folder without any .braid.toml
    let no_config = tmp.path().join("no-config");
    std::fs::create_dir_all(&no_config).unwrap();
    let err = add_project(&no_config, &config_dir).unwrap_err();
    assert!(matches!(err, ViewerError::NoBraidToml { .. }), "expected NoBraidToml, got {err:?}");
}

#[test]
fn add_project_requires_doc_id_in_braid_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_path_buf();

    // Folder with .braid.toml but no doc_id
    let no_doc = tmp.path().join("no-doc");
    std::fs::create_dir_all(&no_doc).unwrap();
    std::fs::write(no_doc.join(".braid.toml"), "sync_server = \"wss://example.com\"\n").unwrap();

    let err = add_project(&no_doc, &config_dir).unwrap_err();
    assert!(matches!(err, ViewerError::MissingDocId { .. }), "expected MissingDocId, got {err:?}");
}

#[test]
fn add_project_rejects_invalid_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_path_buf();
    let bad = tmp.path().join("bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join(".braid.toml"), "this is not toml [[[").unwrap();

    let err = add_project(&bad, &config_dir).unwrap_err();
    assert!(matches!(err, ViewerError::Parse { .. }), "expected Parse, got {err:?}");
}
