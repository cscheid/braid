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
fn create_with_deps_links_atomically() {
    let (_tmp, t) = Skein::new();
    let parent = t.create(&["Parent task"]);

    // one-shot create + link; direction is new-depends-on-target, matching
    // beads' `--deps discovered-from:<parent>`.
    let child = t.create(&["Discovered work", "--deps", &format!("discovered-from:{parent}")]);

    let json = t.show_json(&child);
    let deps = json["dependencies"].as_object().unwrap();
    assert_eq!(deps.len(), 1);
    let dep = deps.values().next().unwrap();
    assert_eq!(dep["depends_on_id"], parent.as_str());
    assert_eq!(dep["type"], "discovered-from");

    // and it surfaces as an incoming edge on the parent
    t.braid()
        .args(["dep", "list", &parent])
        .assert()
        .success()
        .stdout(predicate::str::contains(&child).and(predicate::str::contains("discovered-from")));
}

#[test]
fn create_with_multiple_deps_repeated_and_comma_separated() {
    let (_tmp, t) = Skein::new();
    let a = t.create(&["A"]);
    let b = t.create(&["B"]);

    // repeated flags
    let x = t.create(&["X", "--deps", &format!("blocks:{a}"), "--deps", &format!("related:{b}")]);
    assert_eq!(t.show_json(&x)["dependencies"].as_object().unwrap().len(), 2);

    // comma-separated form, identical effect
    let y = t.create(&["Y", "--deps", &format!("blocks:{a},related:{b}")]);
    assert_eq!(t.show_json(&y)["dependencies"].as_object().unwrap().len(), 2);
}

#[test]
fn create_deps_unknown_type_is_recorded_verbatim() {
    let (_tmp, t) = Skein::new();
    let a = t.create(&["A"]);
    // braid's DependencyType tolerates unknowns (→ Other), like `dep add`.
    let x = t.create(&["X", "--deps", &format!("weird:{a}")]);
    let xj = t.show_json(&x);
    let dep = xj["dependencies"].as_object().unwrap().values().next().unwrap();
    assert_eq!(dep["type"], "weird");
}

#[test]
fn create_deps_bad_format_errors_and_creates_nothing() {
    let (_tmp, t) = Skein::new();
    // format is validated before the session opens, so nothing is created
    t.braid()
        .args(["create", "X", "--deps", "notacolon"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("type").and(predicate::str::contains("notacolon")));

    let out = t.braid().args(["list", "--all", "--json"]).assert().success();
    let all: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 0);
}

#[test]
fn create_deps_missing_target_errors_atomically() {
    let (_tmp, t) = Skein::new();
    // a missing target is rejected like `dep add` (typo guard); critically,
    // the strand is NOT created despite its title being valid.
    t.braid()
        .args(["create", "X", "--deps", "blocks:br-ghost99"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no issue"));

    let out = t.braid().args(["list", "--all", "--json"]).assert().success();
    let all: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 0);
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
fn dep_tree_renders_recursive_parent_child() {
    let (_tmp, t) = Skein::new();
    let a = t.create(&["Epic A", "--type", "epic"]);
    let b = t.create(&["Task B", "--deps", &format!("parent-child:{a}")]);
    let c = t.create(&["Task C", "--deps", &format!("parent-child:{a}")]);
    let d = t.create(&["Task D", "--deps", &format!("parent-child:{c}")]);

    // text tree: A at depth 0, B/C indented under it, D under C
    let out = t.braid().args(["dep", "tree", &a]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with(&a), "root first: {text}");
    // B and C are at one level of indent; D is two levels under C
    assert!(text.contains(&format!("  {b}")), "B indented under A: {text}");
    assert!(text.contains(&format!("  {c}")), "C indented under A: {text}");
    assert!(text.contains(&format!("    {d}")), "D indented under C: {text}");
    assert!(text.contains("[open]"), "statuses shown: {text}");

    // --json: nested structure rooted at A
    let out = t.braid().args(["dep", "tree", &a, "--json"]).assert().success();
    let tree: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(tree["id"], a.as_str());
    let kids: Vec<&str> =
        tree["children"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert!(kids.contains(&b.as_str()) && kids.contains(&c.as_str()));
    let c_node =
        tree["children"].as_array().unwrap().iter().find(|n| n["id"] == c.as_str()).unwrap();
    assert_eq!(c_node["children"][0]["id"], d.as_str());
    assert_eq!(c_node["children"][0]["dep_type"], "parent-child");
}

#[test]
fn dep_tree_breaks_cycles() {
    let (_tmp, t) = Skein::new();
    let a = t.create(&["A"]);
    let b = t.create(&["B", "--deps", &format!("parent-child:{a}")]); // B child of A
    // close the loop: A becomes a child of B
    t.braid().args(["dep", "add", &a, &b, "--type", "parent-child"]).assert().success();

    // must terminate (not recurse forever) and mark the cycle
    let out = t.braid().args(["dep", "tree", &a]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.contains("(cycle)"), "cycle marked: {text}");

    let out = t.braid().args(["dep", "tree", &a, "--json"]).assert().success();
    let tree: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    // A -> B -> A(cycle)
    let b_node = &tree["children"][0];
    assert_eq!(b_node["id"], b.as_str());
    let a_again = &b_node["children"][0];
    assert_eq!(a_again["id"], a.as_str());
    assert_eq!(a_again["cycle"], true);
    assert!(a_again["children"].as_array().unwrap().is_empty());
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
