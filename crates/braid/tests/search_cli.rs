//! e2e tests for `braid search` and `braid agents-info`.

use std::path::PathBuf;

use predicates::prelude::*;

const DEAD_SERVER: &str = "tcp://127.0.0.1:1";

struct Tracker {
    home: PathBuf,
    work: PathBuf,
}

impl Tracker {
    fn new() -> (tempfile::TempDir, Tracker) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        let t = Tracker { home, work };
        t.braid()
            .args(["init", "--name", "search", "--sync-server", DEAD_SERVER])
            .assert()
            .success();
        (tmp, t)
    }

    fn braid(&self) -> assert_cmd::Command {
        let mut c = assert_cmd::Command::cargo_bin("braid").unwrap();
        c.current_dir(&self.work)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", &self.home)
            .env("BRAID_SYNC_TIMEOUT", "0.3");
        c
    }

    fn create(&self, args: &[&str]) -> String {
        let out = self.braid().arg("create").args(args).assert().success();
        String::from_utf8(out.get_output().stdout.clone()).unwrap().trim().to_string()
    }
}

#[test]
fn search_matches_title_description_labels_and_comments() {
    let (_tmp, t) = Tracker::new();
    let by_title = t.create(&["Fix the CRLF handling"]);
    let by_desc = t.create(&["Other thing", "--description", "involves crlf too"]);
    let by_label = t.create(&["Labeled thing", "--label", "crlf-stuff"]);
    let by_comment = t.create(&["Commented thing"]);
    t.braid().args(["comment", &by_comment, "this also mentions CRLF"]).assert().success();
    let unrelated = t.create(&["Nothing to see"]);

    let out = t.braid().args(["search", "crlf"]).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    for id in [&by_title, &by_desc, &by_label, &by_comment] {
        assert!(stdout.contains(id.as_str()), "search should find {id}: {stdout}");
    }
    assert!(!stdout.contains(unrelated.as_str()), "unrelated must not match");
}

#[test]
fn search_is_case_insensitive_and_supports_json() {
    let (_tmp, t) = Tracker::new();
    let id = t.create(&["MIXED case TItle"]);

    let out = t.braid().args(["search", "mixed CASE", "--json"]).assert().success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 1);
    assert_eq!(json[0]["id"], id.as_str());
}

#[test]
fn search_with_no_matches_is_empty_success() {
    let (_tmp, t) = Tracker::new();
    t.create(&["Something"]);
    let out = t.braid().args(["search", "zzzznothing"]).assert().success();
    assert!(String::from_utf8(out.get_output().stdout.clone()).unwrap().trim().is_empty());
}

#[test]
fn agents_info_prints_guide_without_any_config() {
    // agents-info must work anywhere — no tracker, no secret, no network.
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut c = assert_cmd::Command::cargo_bin("braid").unwrap();
    let out = c
        .current_dir(tmp.path())
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env("HOME", &home)
        .arg("agents-info")
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();

    // the essentials an agent needs
    for needle in [
        "# braid",
        "braid ready",
        "braid create",
        "braid close",
        "--json",
        ".braid.toml",
        "BRAID_DOC_ID",
        "secret",
        "skill",
        "braid agents-info",
    ] {
        assert!(stdout.contains(needle), "agents-info should mention {needle:?}");
    }
}

#[test]
fn version_flag_works() {
    let mut c = assert_cmd::Command::cargo_bin("braid").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("braid"));
}
