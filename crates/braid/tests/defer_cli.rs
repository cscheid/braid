//! e2e tests for defer/undefer scheduling commands.
//! All offline (dead server) — sync behavior is covered by tests/sync.rs.
//!
//! Wake semantics are read-time (see braid-core domain tests); here we
//! exercise the CLI surface: status/defer_until transitions, --until
//! parsing, the clearing rules, and ready/list/show presentation.

use std::path::PathBuf;

use predicates::prelude::*;

const DEAD_SERVER: &str = "tcp://127.0.0.1:1";

/// Dates safely in the past / future relative to any test run.
const PAST: &str = "2020-01-01T00:00:00Z";
const FUTURE: &str = "2099-01-01T00:00:00Z";

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
            .args(["init", "--name", "defer", "--sync-server", DEAD_SERVER])
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

    fn ready_ids(&self) -> Vec<String> {
        let out = self.braid().args(["ready", "--json"]).assert().success();
        let rows: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        rows.as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect()
    }
}

#[test]
fn defer_sets_status_and_wake_time() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Park me"]);

    t.braid()
        .args(["defer", &id, "--until", "2099-01-01T09:30:00Z"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&id));

    let json = t.show_json(&id);
    assert_eq!(json["status"], "deferred");
    // normalized to the canonical microsecond-precision UTC form
    assert_eq!(json["defer_until"], "2099-01-01T09:30:00.000000Z");
}

#[test]
fn defer_without_until_is_dateless() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Park me indefinitely"]);
    t.braid().args(["defer", &id]).assert().success();

    let json = t.show_json(&id);
    assert_eq!(json["status"], "deferred");
    assert!(json.get("defer_until").is_none());
    // dateless deferred never wakes on its own
    assert_eq!(t.ready_ids(), Vec::<String>::new());
}

#[test]
fn defer_accepts_bare_date_form() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Date form"]);
    t.braid().args(["defer", &id, "--until", "2099-07-01"]).assert().success();
    assert_eq!(t.show_json(&id)["defer_until"], "2099-07-01T00:00:00.000000Z");
}

#[test]
fn defer_accepts_duration_form() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Duration form"]);

    let before = chrono::Utc::now();
    t.braid().args(["defer", &id, "--until", "7d"]).assert().success();
    let after = chrono::Utc::now();

    let json = t.show_json(&id);
    let until = chrono::DateTime::parse_from_rfc3339(json["defer_until"].as_str().unwrap())
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert!(
        until >= before + chrono::Duration::days(7) && until <= after + chrono::Duration::days(7),
        "7d must resolve to now+7d, got {until}"
    );
}

#[test]
fn defer_rejects_garbage_until_and_changes_nothing() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Untouched"]);

    t.braid()
        .args(["defer", &id, "--until", "soonish"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--until").and(predicate::str::contains("7d")));

    let json = t.show_json(&id);
    assert_eq!(json["status"], "open", "failed defer must not change the strand");
    assert!(json.get("defer_until").is_none());
}

#[test]
fn defer_accepts_multiple_ids() {
    let (_tmp, t) = Skein::new();
    let a = t.create(&["First"]);
    let b = t.create(&["Second"]);

    t.braid()
        .args(["defer", &a, &b, "--until", FUTURE])
        .assert()
        .success()
        .stdout(predicate::str::contains(&a).and(predicate::str::contains(&b)));
    assert_eq!(t.show_json(&a)["status"], "deferred");
    assert_eq!(t.show_json(&b)["status"], "deferred");
}

#[test]
fn defer_closed_strand_errors() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Done already"]);
    t.braid().args(["close", &id]).assert().success();

    t.braid()
        .args(["defer", &id, "--until", FUTURE])
        .assert()
        .failure()
        .stderr(predicate::str::contains("closed").and(predicate::str::contains("reopen")));
    assert_eq!(t.show_json(&id)["status"], "closed");
}

#[test]
fn redefer_updates_or_clears_the_date() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Reschedule me"]);

    t.braid().args(["defer", &id, "--until", "2099-01-01"]).assert().success();
    t.braid().args(["defer", &id, "--until", "2099-06-01"]).assert().success();
    assert_eq!(t.show_json(&id)["defer_until"], "2099-06-01T00:00:00.000000Z");

    // bare re-defer clears the date: now sleeps until explicit undefer
    t.braid().args(["defer", &id]).assert().success();
    assert!(t.show_json(&id).get("defer_until").is_none());
    assert_eq!(t.show_json(&id)["status"], "deferred");
}

#[test]
fn undefer_restores_open_and_clears_date() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Wake me"]);
    t.braid().args(["defer", &id, "--until", FUTURE]).assert().success();

    t.braid().args(["undefer", &id]).assert().success().stdout(predicate::str::contains(&id));

    let json = t.show_json(&id);
    assert_eq!(json["status"], "open");
    assert!(json.get("defer_until").is_none());
}

#[test]
fn undefer_non_deferred_errors() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Already awake"]);

    t.braid()
        .args(["undefer", &id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not deferred").and(predicate::str::contains("open")));
}

#[test]
fn close_clears_defer_until() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Close while parked"]);
    t.braid().args(["defer", &id, "--until", FUTURE]).assert().success();

    t.braid().args(["close", &id, "--reason", "overtaken by events"]).assert().success();
    let json = t.show_json(&id);
    assert_eq!(json["status"], "closed");
    assert!(json.get("defer_until").is_none(), "close must clear defer_until");
}

#[test]
fn update_status_away_from_deferred_clears_date() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Status change"]);
    t.braid().args(["defer", &id, "--until", FUTURE]).assert().success();

    t.braid().args(["update", &id, "--status", "in_progress"]).assert().success();
    let json = t.show_json(&id);
    assert_eq!(json["status"], "in_progress");
    assert!(json.get("defer_until").is_none(), "leaving deferred must clear defer_until");
}

#[test]
fn update_to_deferred_keeps_existing_date_and_unrelated_updates_too() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Keep my date"]);
    t.braid().args(["defer", &id, "--until", FUTURE]).assert().success();

    // re-asserting deferred is not a status change away — date survives
    t.braid().args(["update", &id, "--status", "deferred"]).assert().success();
    assert!(t.show_json(&id)["defer_until"].is_string());

    // unrelated field updates also leave the date alone
    t.braid().args(["update", &id, "--priority", "0"]).assert().success();
    assert!(t.show_json(&id)["defer_until"].is_string());
}

#[test]
fn update_status_deferred_still_works_bare() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Old style"]);
    t.braid().args(["update", &id, "--status", "deferred"]).assert().success();
    let json = t.show_json(&id);
    assert_eq!(json["status"], "deferred");
    assert!(json.get("defer_until").is_none());
}

#[test]
fn ready_wakes_expired_defer_and_sleeps_future_ones() {
    let (_tmp, t) = Skein::new();
    let due = t.create(&["Due now"]);
    let later = t.create(&["Due much later"]);
    t.braid().args(["defer", &due, "--until", PAST]).assert().success();
    t.braid().args(["defer", &later, "--until", FUTURE]).assert().success();

    assert_eq!(t.ready_ids(), vec![due.clone()]);

    // the wake is computed, not written: the strand still reads deferred
    assert_eq!(t.show_json(&due)["status"], "deferred");
}

#[test]
fn show_and_list_render_the_wake_time() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Visible wake time"]);
    t.braid().args(["defer", &id, "--until", "2099-07-01"]).assert().success();

    t.braid()
        .args(["show", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("wakes:     2099-07-01T00:00:00.000000Z"));

    t.braid()
        .args(["list", "--status", "deferred"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[wakes 2099-07-01T00:00:00.000000Z]"));
}

#[test]
fn defer_resolves_id_fragments() {
    let (_tmp, t) = Skein::new();
    let id = t.create(&["Fragment me", "--slug", "frag"]);
    let fragment = &id[id.len() - 6..];
    t.braid().args(["defer", fragment, "--until", FUTURE]).assert().success();
    assert_eq!(t.show_json(&id)["status"], "deferred");
}
