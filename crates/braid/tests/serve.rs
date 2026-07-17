//! End-to-end tests for `braid serve` — the loom, a standalone sync
//! server other braid clients collaborate through (issue #22, strand
//! br-loom-3qm0ze53).
//!
//! Unlike tests/sync.rs, which runs an in-process samod repo as the
//! server, these tests spawn the real `braid serve` binary and talk to it
//! exactly the way users do: parse the listening URL it prints, point
//! clones at it via `.braid.toml`, and sync.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use predicates::prelude::*;
use tokio::io::{AsyncBufReadExt, BufReader};

/// A spawned `braid serve` process. Killed on drop so a failing test
/// never leaks a listener.
struct Loom {
    child: tokio::process::Child,
    url: String,
}

impl Loom {
    /// Spawn `braid serve --port 0 <extra>` and parse the listening URL
    /// from its first stdout line (`loom listening on ws://…`).
    async fn start(extra: &[&str]) -> Loom {
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_braid"));
        cmd.arg("serve")
            .args(["--port", "0"])
            .args(extra)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for key in ["SystemRoot", "SystemDrive", "TEMP", "TMP"] {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        let mut child = cmd.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut lines = BufReader::new(stdout).lines();
        let line = tokio::time::timeout(std::time::Duration::from_secs(30), lines.next_line())
            .await
            .expect("braid serve did not print its listening line in time")
            .unwrap()
            .expect("braid serve exited before printing its listening line");
        let url = line
            .rsplit(' ')
            .next()
            .filter(|url| url.starts_with("ws://"))
            .unwrap_or_else(|| panic!("unparseable listening line: {line:?}"))
            .to_string();
        Loom { child, url }
    }

    async fn stop(mut self) {
        self.child.kill().await.unwrap();
    }
}

/// A simulated machine: its own HOME (and therefore its own braid cache)
/// and its own working directory. Mirrors tests/sync.rs.
struct Clone_ {
    home: PathBuf,
    work: PathBuf,
}

impl Clone_ {
    fn new(root: &Path, name: &str) -> Clone_ {
        let home = root.join(format!("{name}-home"));
        let work = root.join(format!("{name}-work"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        Clone_ { home, work }
    }

    fn braid(&self) -> assert_cmd::Command {
        let mut c = assert_cmd::Command::cargo_bin("braid").unwrap();
        c.current_dir(&self.work)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", &self.home)
            .env("BRAID_SYNC_TIMEOUT", "10");
        // Windows: env_clear() strips SystemRoot, without which Winsock
        // can't initialize. No-op on Unix.
        for key in ["SystemRoot", "SystemDrive", "TEMP", "TMP"] {
            if let Ok(val) = std::env::var(key) {
                c.env(key, val);
            }
        }
        c
    }

    fn write_secret(&self, doc_id: &str, server_url: &str) {
        std::fs::write(
            self.work.join(".braid.toml"),
            format!("doc_id = \"{doc_id}\"\nsync_server = \"{server_url}\"\n"),
        )
        .unwrap();
    }

    fn doc_id(&self) -> String {
        let secret = std::fs::read_to_string(self.work.join(".braid.toml")).unwrap();
        let parsed: toml::Value = toml::from_str(&secret).unwrap();
        parsed["doc_id"].as_str().unwrap().to_string()
    }
}

/// Every file under `root`, as paths relative to it (skipping in-flight
/// atomic-write temp files).
fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if !entry.file_name().to_string_lossy().starts_with(".tmp-") {
                out.push(path.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    out
}

/// Wait until the loom has persisted at least one storage file.
async fn wait_for_files(root: &Path) {
    for _ in 0..100 {
        if !files_under(root).is_empty() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("loom wrote nothing under {} within 10s", root.display());
}

// ---------------------------------------------------------------------------
// CLI surface
// ---------------------------------------------------------------------------

#[test]
fn serve_requires_an_explicit_storage_choice() {
    // Neither flag: refused, and the error names both options.
    assert_cmd::Command::cargo_bin("braid").unwrap().arg("serve").assert().failure().stderr(
        predicate::str::contains("--data-dir").and(predicate::str::contains("--in-memory")),
    );

    // Both flags: also refused.
    let tmp = tempfile::tempdir().unwrap();
    assert_cmd::Command::cargo_bin("braid")
        .unwrap()
        .args(["serve", "--in-memory", "--data-dir"])
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("--in-memory").and(predicate::str::contains("--data-dir")),
        );
}

#[test]
fn serve_help_documents_the_defaults() {
    assert_cmd::Command::cargo_bin("braid")
        .unwrap()
        .args(["serve", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("3030")
                .and(predicate::str::contains("127.0.0.1"))
                .and(predicate::str::contains("--data-dir"))
                .and(predicate::str::contains("--in-memory")),
        );
}

// ---------------------------------------------------------------------------
// Syncing through the loom
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn two_clones_converge_through_the_loom() {
    let tmp = tempfile::tempdir().unwrap();
    let loom = Loom::start(&["--in-memory"]).await;

    let a = Clone_::new(tmp.path(), "a");
    a.braid().args(["init", "--name", "loomed", "--sync-server", &loom.url]).assert().success();
    let doc_id = a.doc_id();

    let b = Clone_::new(tmp.path(), "b");
    b.write_secret(&doc_id, &loom.url);

    a.braid().args(["create", "issue from A"]).assert().success();
    b.braid().args(["create", "issue from B"]).assert().success();

    for clone in [&a, &b] {
        let out = clone.braid().args(["list", "--json"]).assert().success();
        let issues: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        let titles: Vec<&str> =
            issues.as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
        assert!(titles.contains(&"issue from A"), "missing A's issue: {titles:?}");
        assert!(titles.contains(&"issue from B"), "missing B's issue: {titles:?}");
    }

    loom.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn loom_persists_skeins_across_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("loom-data");
    let data_dir_str = data_dir.to_str().unwrap().to_string();

    let loom = Loom::start(&["--data-dir", &data_dir_str]).await;
    let a = Clone_::new(tmp.path(), "a");
    a.braid().args(["init", "--name", "durable", "--sync-server", &loom.url]).assert().success();
    let doc_id = a.doc_id();
    a.braid().args(["create", "survives restarts"]).assert().success();
    a.braid().arg("sync").assert().success();
    wait_for_files(&data_dir).await;
    loom.stop().await;

    // The loom's disk must never hold the doc id (a bearer secret) in the
    // clear — keys are hashed (design decision D-serve-2). The storage
    // layout splays the first two characters of the first component into a
    // directory, so check for the remainder as well as the whole id.
    let needle = &doc_id[2..];
    for file in files_under(&data_dir) {
        let path = file.to_string_lossy();
        assert!(
            !path.contains(needle) && !path.contains(&doc_id),
            "doc id leaked into loom storage path: {path}"
        );
    }

    // A restarted loom serves the skein to a brand-new clone from disk.
    let loom = Loom::start(&["--data-dir", &data_dir_str]).await;
    let b = Clone_::new(tmp.path(), "b");
    b.write_secret(&doc_id, &loom.url);
    b.braid().arg("list").assert().success().stdout(predicate::str::contains("survives restarts"));

    loom.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn in_memory_loom_forgets_on_restart() {
    let tmp = tempfile::tempdir().unwrap();

    let loom = Loom::start(&["--in-memory"]).await;
    let a = Clone_::new(tmp.path(), "a");
    a.braid().args(["init", "--name", "fleeting", "--sync-server", &loom.url]).assert().success();
    let doc_id = a.doc_id();
    a.braid().args(["create", "gone after restart"]).assert().success();
    loom.stop().await;

    // Restart: the skein is gone, so a fresh clone (empty cache) cannot
    // fetch it and must fail rather than silently show an empty skein.
    let loom = Loom::start(&["--in-memory"]).await;
    let b = Clone_::new(tmp.path(), "b");
    b.write_secret(&doc_id, &loom.url);
    b.braid().arg("list").assert().failure();

    loom.stop().await;
}
