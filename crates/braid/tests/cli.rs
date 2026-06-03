//! End-to-end CLI tests: init/create/show/list against the local cache only
//! (no network involved anywhere — Phase 1).
//!
//! Every invocation runs with a cleared environment and HOME pointed at a
//! tempdir, so the user's real config, cache, and git identity never leak in
//! (author deterministically resolves to "unknown").

use std::path::Path;

use predicates::prelude::*;

fn braid(cwd: &Path, home: &Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("braid").unwrap();
    c.current_dir(cwd)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env("HOME", home);
    c
}

/// init in `dir`, returning the doc id parsed from `.braid.toml`.
fn init_tracker(dir: &Path, home: &Path) -> String {
    braid(dir, home).args(["init", "--name", "test-project"]).assert().success();
    let secret = std::fs::read_to_string(dir.join(".braid.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&secret).unwrap();
    parsed["doc_id"].as_str().unwrap().to_string()
}

/// run `braid create` and return the printed issue id.
fn create_issue(dir: &Path, home: &Path, args: &[&str]) -> String {
    let out = braid(dir, home).arg("create").args(args).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let id = stdout.trim().to_string();
    assert!(id.starts_with("br-"), "create should print the new id, got {stdout:?}");
    id
}

#[test]
fn init_writes_secret_file_and_warns_about_gitignore() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    braid(&work, &home)
        .args(["init", "--name", "test-project"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gitignore"));

    let secret_path = work.join(".braid.toml");
    let secret = std::fs::read_to_string(&secret_path).unwrap();
    let parsed: toml::Value = toml::from_str(&secret).unwrap();
    assert!(!parsed["doc_id"].as_str().unwrap().is_empty());
    assert_eq!(parsed["sync_server"].as_str().unwrap(), "wss://sync.automerge.org");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&secret_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "secret file must be owner-only");
    }
}

#[test]
fn init_twice_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    init_tracker(&work, &home);
    braid(&work, &home)
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains(".braid.toml"));
}

#[test]
fn init_print_only_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    braid(&work, &home)
        .args(["init", "--print-only"])
        .assert()
        .success()
        .stdout(predicate::str::contains("doc_id = "));
    assert!(!work.join(".braid.toml").exists());
}

#[test]
fn create_show_list_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    init_tracker(&work, &home);

    let id = create_issue(
        &work,
        &home,
        &[
            "Fix the frobnicator",
            "--description",
            "It frobs when it should nicate.",
            "--priority",
            "1",
            "--label",
            "bug",
            "--label",
            "urgent",
        ],
    );

    // list shows it
    braid(&work, &home)
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("Fix the frobnicator").and(predicate::str::contains(&id)));

    // show --json round-trips the fields
    let out = braid(&work, &home).args(["show", &id, "--json"]).assert().success();
    let json: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["id"], id.as_str());
    assert_eq!(json["title"], "Fix the frobnicator");
    assert_eq!(json["description"], "It frobs when it should nicate.");
    assert_eq!(json["status"], "open");
    assert_eq!(json["priority"], 1);
    assert_eq!(json["issue_type"], "task");
    assert_eq!(json["created_by"], "unknown");
    assert_eq!(json["labels"], serde_json::json!(["bug", "urgent"]));

    // show by unique id fragment (the random suffix)
    let fragment = id.strip_prefix("br-").unwrap();
    braid(&work, &home)
        .args(["show", fragment])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fix the frobnicator"));
}

#[test]
fn create_json_outputs_full_issue() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    init_tracker(&work, &home);

    let out = braid(&work, &home)
        .args(["create", "JSON output test", "--json"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["title"], "JSON output test");
    assert!(json["id"].as_str().unwrap().starts_with("br-"));
}

#[test]
fn slug_appears_in_generated_id() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    init_tracker(&work, &home);

    let id = create_issue(&work, &home, &["Slugged issue", "--slug", "My Slug!"]);
    assert!(id.starts_with("br-my-slug-"), "got {id}");
}

#[test]
fn list_status_filter_and_json() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    init_tracker(&work, &home);

    create_issue(&work, &home, &["First issue"]);
    create_issue(&work, &home, &["Second issue", "--priority", "0"]);

    // --json gives an array sorted by priority
    let out = braid(&work, &home).args(["list", "--json"]).assert().success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["title"], "Second issue", "priority 0 sorts first");

    // status filtering
    let out = braid(&work, &home)
        .args(["list", "--status", "closed", "--json"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[test]
fn missing_config_gives_helpful_error() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    braid(&work, &home)
        .args(["create", "doomed"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("BRAID_DOC_ID")
                .and(predicate::str::contains("braid init")),
        );
}

#[test]
fn unknown_doc_id_not_in_cache_mentions_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    // a syntactically valid doc id that exists in no cache: generate one by
    // initializing a *different* directory, then pointing at a fresh HOME.
    let other = tmp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let foreign_doc_id = init_tracker(&other, &home);

    let fresh_home = tmp.path().join("home2");
    std::fs::create_dir_all(&fresh_home).unwrap();
    std::fs::write(
        work.join(".braid.toml"),
        format!("doc_id = \"{foreign_doc_id}\"\n"),
    )
    .unwrap();

    braid(&work, &fresh_home)
        .args(["create", "doomed"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cache"));
}

#[test]
fn invalid_doc_id_format_is_a_clear_error() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join(".braid.toml"), "doc_id = \"not a doc id!\"\n").unwrap();

    braid(&work, &home)
        .args(["list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("doc_id"));
}

#[test]
fn show_unknown_and_ambiguous_ids_error() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    init_tracker(&work, &home);

    create_issue(&work, &home, &["First"]);
    create_issue(&work, &home, &["Second"]);

    braid(&work, &home)
        .args(["show", "zzzznope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no issue"));

    // "br-" matches both issues
    braid(&work, &home)
        .args(["show", "br-"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("ambiguous"));
}

#[test]
fn init_join_adopts_existing_doc_id() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    // Create a tracker elsewhere to obtain a valid doc id that's in cache.
    let other = tmp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let doc_id = init_tracker(&other, &home);

    braid(&work, &home)
        .args(["init", "--join", &doc_id])
        .assert()
        .success();

    let secret = std::fs::read_to_string(work.join(".braid.toml")).unwrap();
    assert!(secret.contains(&doc_id));

    // Same HOME → same cache → the joined tracker is usable immediately.
    create_issue(&work, &home, &["Issue in joined tracker"]);
    braid(&other, &home)
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("Issue in joined tracker"));
}
