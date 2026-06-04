//! e2e tests for `braid delete` (strand br-delete-cmd).
//!
//! Deletion is the sharpest mutation braid has: the merge tests pin that
//! a delete wins over concurrent edits to the same strand. The porcelain
//! therefore guards the case where other strands still reference the
//! target (dependents) behind --force, mirroring close's open-children
//! protection. Dangling edges left behind are contract-legal (they never
//! block) and visible in `dep list` as [missing!].

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
        t.braid().args(["init", "--name", "del", "--sync-server", DEAD_SERVER]).assert().success();
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

    fn list_ids(&self) -> Vec<String> {
        let out = self.braid().args(["list", "--json"]).assert().success();
        let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        json.as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect()
    }
}

#[test]
fn delete_removes_the_strand() {
    let (_tmp, t) = Skein::new();
    let doomed = t.create(&["Doomed strand"]);
    let survivor = t.create(&["Survivor"]);

    // resolve by unique fragment, like every other command
    let fragment = doomed.strip_prefix("br-").unwrap();
    t.braid()
        .args(["delete", fragment])
        .assert()
        .success()
        .stdout(predicate::str::contains(&doomed));

    assert_eq!(t.list_ids(), vec![survivor]);
    t.braid()
        .args(["show", &doomed])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no issue"));
}

#[test]
fn delete_accepts_multiple_ids() {
    let (_tmp, t) = Skein::new();
    let a = t.create(&["First"]);
    let b = t.create(&["Second"]);
    let keep = t.create(&["Keeper"]);

    t.braid().args(["delete", &a, &b]).assert().success();
    assert_eq!(t.list_ids(), vec![keep]);
}

#[test]
fn delete_unknown_id_errors_without_deleting_anything() {
    let (_tmp, t) = Skein::new();
    let a = t.create(&["Here"]);

    t.braid()
        .args(["delete", &a, "zzz-nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no issue"));
    assert_eq!(t.list_ids(), vec![a], "failed delete must be atomic");
}

#[test]
fn delete_with_dependents_requires_force() {
    let (_tmp, t) = Skein::new();
    let target = t.create(&["Depended upon"]);
    let dependent = t.create(&["Depends on it"]);
    t.braid().args(["dep", "add", &dependent, &target]).assert().success();

    // refused, naming the dependent
    t.braid()
        .args(["delete", &target])
        .assert()
        .failure()
        .stderr(predicate::str::contains(&dependent).and(predicate::str::contains("--force")));
    assert_eq!(t.list_ids().len(), 2);

    // --force deletes and notes the dangling references
    t.braid()
        .args(["delete", &target, "--force"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dangling"));
    assert_eq!(t.list_ids(), vec![dependent.clone()]);

    // the dangling edge is visible but harmless: dependent is still ready
    t.braid()
        .args(["dep", "list", &dependent])
        .assert()
        .success()
        .stdout(predicate::str::contains("missing!"));
    t.braid().args(["ready"]).assert().success().stdout(predicate::str::contains(&dependent));
}

#[test]
fn deleting_strands_in_the_same_invocation_needs_no_force() {
    // deleting a target together with all its dependents is not dangling
    let (_tmp, t) = Skein::new();
    let target = t.create(&["Target"]);
    let dependent = t.create(&["Dependent"]);
    t.braid().args(["dep", "add", &dependent, &target]).assert().success();

    t.braid().args(["delete", &target, &dependent]).assert().success();
    assert!(t.list_ids().is_empty());
}
