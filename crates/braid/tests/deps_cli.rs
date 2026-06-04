//! e2e tests for dependency commands and ready/blocked listings.
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
        t.braid().args(["init", "--name", "deps", "--sync-server", DEAD_SERVER]).assert().success();
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
fn dep_add_list_remove_round_trip() {
    let (_tmp, t) = Skein::new();
    let a = t.create(&["Issue A"]);
    let b = t.create(&["Issue B"]);

    // default type is blocks
    t.braid().args(["dep", "add", &a, &b]).assert().success();

    let json = t.show_json(&a);
    let deps = json["dependencies"].as_object().unwrap();
    assert_eq!(deps.len(), 1);
    let dep = deps.values().next().unwrap();
    assert_eq!(dep["depends_on_id"], b.as_str());
    assert_eq!(dep["type"], "blocks");

    // dep list shows both directions
    t.braid()
        .args(["dep", "list", &a])
        .assert()
        .success()
        .stdout(predicate::str::contains(&b).and(predicate::str::contains("blocks")));
    t.braid().args(["dep", "list", &b]).assert().success().stdout(predicate::str::contains(&a)); // incoming

    t.braid().args(["dep", "remove", &a, &b]).assert().success();
    let json = t.show_json(&a);
    assert!(json.get("dependencies").is_none(), "no deps left after removal");
}

#[test]
fn dep_add_validates_targets_and_self_edges() {
    let (_tmp, t) = Skein::new();
    let a = t.create(&["Lonely"]);

    t.braid()
        .args(["dep", "add", &a, "br-ghost99"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no issue"));

    t.braid()
        .args(["dep", "add", &a, &a])
        .assert()
        .failure()
        .stderr(predicate::str::contains("itself"));
}

#[test]
fn dep_add_warns_on_cycle_but_allows_it() {
    let (_tmp, t) = Skein::new();
    let a = t.create(&["A"]);
    let b = t.create(&["B"]);

    t.braid().args(["dep", "add", &a, &b]).assert().success();
    // closing the loop: allowed (CRDT merges can create cycles anyway,
    // so tooling must handle them), but warned about.
    t.braid()
        .args(["dep", "add", &b, &a])
        .assert()
        .success()
        .stderr(predicate::str::contains("cycle"));

    // and `dep cycles` reports it
    t.braid()
        .args(["dep", "cycles"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&a).and(predicate::str::contains(&b)));
}

#[test]
fn dep_cycles_silent_when_acyclic() {
    let (_tmp, t) = Skein::new();
    let a = t.create(&["A"]);
    let b = t.create(&["B"]);
    t.braid().args(["dep", "add", &a, &b]).assert().success();

    let out = t.braid().args(["dep", "cycles"]).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("no cycles") || stdout.trim().is_empty(),
        "expected no cycle output, got {stdout:?}"
    );
}

#[test]
fn ready_and_blocked_listings() {
    let (_tmp, t) = Skein::new();
    let blocked = t.create(&["The blocked one"]);
    let blocker = t.create(&["The blocker"]);
    let free = t.create(&["The free one"]);
    t.braid().args(["dep", "add", &blocked, &blocker]).assert().success();

    // ready: blocker + free, not blocked
    let out = t.braid().args(["ready"]).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains(&blocker));
    assert!(stdout.contains(&free));
    assert!(!stdout.contains(&blocked));

    // blocked: shows the blocked issue and its blocker
    t.braid()
        .args(["blocked"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&blocked).and(predicate::str::contains(&blocker)));

    // closing the blocker frees the blocked issue
    t.braid().args(["close", &blocker]).assert().success();
    t.braid().args(["ready"]).assert().success().stdout(predicate::str::contains(&blocked));
    let out = t.braid().args(["blocked"]).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(!stdout.contains(&blocked));
}

#[test]
fn ready_supports_json() {
    let (_tmp, t) = Skein::new();
    t.create(&["Only issue"]);
    let out = t.braid().args(["ready", "--json"]).assert().success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 1);
    assert_eq!(json[0]["title"], "Only issue");
}

#[test]
fn ready_filters_by_label_assignee_and_type() {
    let (_tmp, t) = Skein::new();
    // `blocked` carries the filter label but is dependency-blocked: the
    // filters narrow the ready set, they never resurrect blocked strands.
    let blocked = t.create(&["Blocked", "--label", "x"]);
    let blocker = t.create(&["Blocker", "--label", "x", "--assignee", "alice", "--type", "bug"]);
    let plain = t.create(&["Plain"]);
    t.braid().args(["dep", "add", &blocked, &blocker]).assert().success();

    let ids = |args: &[&str]| -> Vec<String> {
        let mut full = vec!["ready", "--json"];
        full.extend_from_slice(args);
        let out = t.braid().args(&full).assert().success();
        let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        json.as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect()
    };

    // unfiltered baseline: everything awake and unblocked
    assert_eq!(ids(&[]), vec![blocker.clone(), plain.clone()]);

    // each filter narrows the ready set (and blocked stays excluded)
    assert_eq!(ids(&["--label", "x"]), vec![blocker.clone()]);
    assert_eq!(ids(&["--assignee", "alice"]), vec![blocker.clone()]);
    assert_eq!(ids(&["--type", "bug"]), vec![blocker.clone()]);

    // filters compose; a non-matching combination is empty
    assert_eq!(ids(&["--label", "x", "--type", "bug"]), vec![blocker.clone()]);
    assert_eq!(ids(&["--label", "x", "--assignee", "bob"]), Vec::<String>::new());
}

#[test]
fn parent_child_does_not_block_child_in_ready() {
    let (_tmp, t) = Skein::new();
    let epic = t.create(&["Epic", "--type", "epic"]);
    let child = t.create(&["Child task"]);
    t.braid().args(["dep", "add", &child, &epic, "--type", "parent-child"]).assert().success();

    let out = t.braid().args(["ready"]).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains(&child), "child of open epic must be ready");
    assert!(stdout.contains(&epic));
}
