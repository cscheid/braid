//! e2e tests for issue mutation commands: update, close, reopen, comment.
//! All offline (dead server) — sync behavior is covered by tests/sync.rs.

use std::path::PathBuf;

use predicates::prelude::*;

const DEAD_SERVER: &str = "tcp://127.0.0.1:1";

struct Skein {
    home: PathBuf,
    work: PathBuf,
}

impl Skein {
    fn new() -> (tempfile::TempDir, Skein) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        let t = Skein { home, work };
        t.braid()
            .args(["init", "--name", "ops", "--sync-server", DEAD_SERVER])
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

    fn show_json(&self, id: &str) -> serde_json::Value {
        let out = self.braid().args(["show", id, "--json"]).assert().success();
        serde_json::from_slice(&out.get_output().stdout).unwrap()
    }
}

#[test]
fn update_changes_fields_and_bumps_updated_at() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Original title", "--description", "original description"]);
    let before = t.show_json(&id);
    assert_eq!(before["updated_at"], before["created_at"]);

    t.braid()
        .args([
            "update",
            &id,
            "--title",
            "New title",
            "--priority",
            "0",
            "--status",
            "in_progress",
            "--type",
            "bug",
            "--assignee",
            "agent-7",
            "--add-label",
            "first",
            "--add-label",
            "second",
            "--notes",
            "some notes",
        ])
        .assert()
        .success();

    let after = t.show_json(&id);
    assert_eq!(after["title"], "New title");
    assert_eq!(after["priority"], 0);
    assert_eq!(after["status"], "in_progress");
    assert_eq!(after["issue_type"], "bug");
    assert_eq!(after["assignee"], "agent-7");
    assert_eq!(after["labels"], serde_json::json!(["first", "second"]));
    assert_eq!(after["notes"], "some notes");
    assert_eq!(after["description"], "original description", "untouched field survives");
    assert_ne!(after["updated_at"], before["updated_at"], "updated_at must change");
    assert_eq!(after["created_at"], before["created_at"], "created_at must not");
}

#[test]
fn update_empty_string_clears_optional_fields() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Has extras", "--description", "to be removed", "--assignee", "x"]);

    t.braid()
        .args(["update", &id, "--description", "", "--assignee", ""])
        .assert()
        .success();

    let after = t.show_json(&id);
    assert!(after.get("description").is_none(), "empty string clears description");
    assert!(after.get("assignee").is_none(), "empty string clears assignee");
}

#[test]
fn update_remove_label() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Labeled", "--label", "keep", "--label", "drop"]);

    t.braid().args(["update", &id, "--remove-label", "drop"]).assert().success();
    assert_eq!(t.show_json(&id)["labels"], serde_json::json!(["keep"]));
}

#[test]
fn update_unknown_id_errors() {
    let (_tmp, t) = Skein::new();
    t.create(&["Exists"]);
    t.braid()
        .args(["update", "zzz-nope", "--title", "x"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no issue"));
}

#[test]
fn close_sets_fields_and_filters_from_open() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Done soon"]);

    t.braid()
        .args(["close", &id, "--reason", "fixed in tests"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&id));

    let json = t.show_json(&id);
    assert_eq!(json["status"], "closed");
    assert_eq!(json["close_reason"], "fixed in tests");
    assert!(json["closed_at"].is_string());

    let out = t.braid().args(["list", "--status", "open", "--json"]).assert().success();
    let open: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(open.as_array().unwrap().len(), 0);
}

#[test]
fn close_accepts_multiple_ids() {
    let (_tmp, t) = Skein::new();
    let a = t.create(&["First"]);
    let b = t.create(&["Second"]);

    t.braid().args(["close", &a, &b, "--reason", "batch"]).assert().success();
    assert_eq!(t.show_json(&a)["status"], "closed");
    assert_eq!(t.show_json(&b)["status"], "closed");
}

#[test]
fn close_with_open_children_is_refused_without_force() {
    let (_tmp, t) = Skein::new();
    let epic = t.create(&["The epic", "--type", "epic"]);
    let child = t.create(&["The child"]);
    t.braid()
        .args(["dep", "add", &child, &epic, "--type", "parent-child"])
        .assert()
        .success();

    // refused while the child is open
    t.braid()
        .args(["close", &epic])
        .assert()
        .failure()
        .stderr(predicate::str::contains("open child"));

    // --force overrides
    t.braid().args(["close", &epic, "--force"]).assert().success();
    t.braid().args(["reopen", &epic]).assert().success();

    // closing the child first also unlocks it
    t.braid().args(["close", &child]).assert().success();
    t.braid().args(["close", &epic]).assert().success();
    assert_eq!(t.show_json(&epic)["status"], "closed");
}

#[test]
fn reopen_clears_close_fields() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Round trip"]);
    t.braid().args(["close", &id, "--reason", "oops"]).assert().success();
    t.braid().args(["reopen", &id]).assert().success();

    let json = t.show_json(&id);
    assert_eq!(json["status"], "open");
    assert!(json.get("closed_at").is_none());
    assert!(json.get("close_reason").is_none());
}

#[test]
fn comments_append_and_render() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Discussed"]);

    let out = t.braid().args(["comment", &id, "first comment"]).assert().success();
    let cid = String::from_utf8(out.get_output().stdout.clone()).unwrap().trim().to_string();
    assert!(cid.starts_with("c-"), "comment id printed, got {cid:?}");

    t.braid().args(["comment", &id, "second comment"]).assert().success();

    let json = t.show_json(&id);
    let comments = json["comments"].as_object().unwrap();
    assert_eq!(comments.len(), 2);
    let texts: Vec<&str> =
        comments.values().map(|c| c["text"].as_str().unwrap()).collect();
    assert!(texts.contains(&"first comment"));
    assert!(texts.contains(&"second comment"));
    for c in comments.values() {
        assert_eq!(c["author"], "unknown");
    }

    // human-readable show includes comments
    t.braid()
        .args(["show", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("first comment").and(predicate::str::contains("second comment")));
}
