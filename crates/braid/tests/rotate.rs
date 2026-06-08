//! e2e tests for skein rotation (strand br-doc-growth; plan
//! claude-notes/plans/2026/06/04/braid-rotate.md).
//!
//! Real binary invocations against an in-process sync server, exercising
//! both modes: compact (forwarding pointer, `--adopt` follows it) and
//! revoke (no pointer, out-of-band redistribution), plus straggler
//! detection and the offline refusal.

use std::path::{Path, PathBuf};

use predicates::prelude::*;
use samod::storage::InMemoryStorage;

const DEAD_SERVER: &str = "tcp://127.0.0.1:1";

struct TestServer {
    url: String,
    repo: samod::Repo,
    accept_task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    async fn start() -> TestServer {
        let repo = samod::Repo::build_tokio().with_storage(InMemoryStorage::new()).load().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("tcp://{addr}");
        let acceptor = repo.make_acceptor(samod::Url::parse(&url).unwrap()).unwrap();
        let accept_task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let _ = acceptor.accept_tokio_io(stream);
            }
        });
        TestServer { url, repo, accept_task }
    }

    async fn stop(self) {
        self.accept_task.abort();
        self.repo.stop().await;
    }
}

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
        // can't initialize and the spawned process can't reach the sync
        // server. No-op on Unix (these vars don't exist there).
        for key in ["SystemRoot", "SystemDrive", "TEMP", "TMP"] {
            if let Ok(val) = std::env::var(key) {
                c.env(key, val);
            }
        }
        c
    }

    fn doc_id(&self) -> String {
        let secret = std::fs::read_to_string(self.work.join(".braid.toml")).unwrap();
        let parsed: toml::Value = toml::from_str(&secret).unwrap();
        parsed["doc_id"].as_str().unwrap().to_string()
    }

    fn write_secret(&self, doc_id: &str, server_url: &str) {
        std::fs::write(
            self.work.join(".braid.toml"),
            format!("doc_id = \"{doc_id}\"\nsync_server = \"{server_url}\"\n"),
        )
        .unwrap();
    }

    fn create(&self, title: &str) -> String {
        let out = self.braid().args(["create", title]).assert().success();
        String::from_utf8(out.get_output().stdout.clone()).unwrap().trim().to_string()
    }

    fn list_titles(&self) -> Vec<String> {
        let out = self.braid().args(["list", "--json"]).assert().success();
        let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        json.as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap().to_string()).collect()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn compact_rotation_carries_state_and_stale_clones_adopt() {
    let tmp = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;

    // A: init, create strands.
    let a = Clone_::new(tmp.path(), "a");
    a.braid().args(["init", "--name", "rot", "--sync-server", &server.url]).assert().success();
    let old_id = a.doc_id();
    a.create("first strand");
    a.create("second strand");

    // C: a second clone of the *old* skein, synced once.
    let c = Clone_::new(tmp.path(), "c");
    c.write_secret(&old_id, &server.url);
    assert_eq!(c.list_titles().len(), 2);

    // A rotates (compact).
    let out = a.braid().arg("rotate").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("rotated"), "{stdout}");
    assert!(stdout.contains("2 strand"), "should report carried strands: {stdout}");

    // A's secret file now points at a different doc id, and braid works.
    let new_id = a.doc_id();
    assert_ne!(new_id, old_id);
    let mut titles = a.list_titles();
    titles.sort();
    assert_eq!(titles, ["first strand", "second strand"]);

    // The full ids never appear in rotate output.
    assert!(!stdout.contains(&old_id) && !stdout.contains(&new_id), "{stdout}");

    // A fresh clone of the *new* skein sees everything.
    let b = Clone_::new(tmp.path(), "b");
    b.write_secret(&new_id, &server.url);
    assert_eq!(b.list_titles().len(), 2);

    // The stale clone C is refused with adoption instructions...
    c.braid()
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("rotated").and(predicate::str::contains("--adopt")));

    // ...and adopts successfully.
    c.braid().args(["rotate", "--adopt"]).assert().success();
    assert_eq!(c.doc_id(), new_id, "adopt must rewrite .braid.toml");
    assert_eq!(c.list_titles().len(), 2);

    // Rotating the already-rotated old skein is refused by the same check.
    let d = Clone_::new(tmp.path(), "d");
    d.write_secret(&old_id, &server.url);
    d.braid().arg("rotate").assert().failure().stderr(predicate::str::contains("rotated"));

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn revoke_rotation_leaves_no_pointer() {
    let tmp = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;

    let a = Clone_::new(tmp.path(), "a");
    a.braid().args(["init", "--name", "rev", "--sync-server", &server.url]).assert().success();
    let old_id = a.doc_id();
    a.create("sensitive strand");

    let out = a.braid().args(["rotate", "--revoke"]).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("revoke") || stdout.contains("out-of-band"), "{stdout}");
    let new_id = a.doc_id();
    assert_ne!(new_id, old_id);
    assert_eq!(a.list_titles(), ["sensitive strand"]);

    // A stale clone is told about the rotation but NOT pointed anywhere,
    // and --adopt cannot follow (there is nothing to follow).
    let c = Clone_::new(tmp.path(), "c");
    c.write_secret(&old_id, &server.url);
    let out = c.braid().arg("list").assert().failure();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("rotated"), "{stderr}");
    assert!(!stderr.contains("--adopt"), "revoke error must not suggest --adopt:\n{stderr}");
    assert!(!stderr.contains(&new_id), "the new id must be nowhere near the old skein:\n{stderr}");

    c.braid()
        .args(["rotate", "--adopt"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not recorded").or(predicate::str::contains("revoke")));

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn adopt_detects_stragglers() {
    let tmp = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;

    let a = Clone_::new(tmp.path(), "a");
    a.braid().args(["init", "--name", "strag", "--sync-server", &server.url]).assert().success();
    let old_id = a.doc_id();
    a.create("pre-rotation strand");

    // C clones and syncs the old skein.
    let c = Clone_::new(tmp.path(), "c");
    c.write_secret(&old_id, &server.url);
    assert_eq!(c.list_titles().len(), 1);

    // A rotates while C is offline...
    a.braid().arg("rotate").assert().success();

    // ...and C, still unaware, writes to the old skein offline.
    c.braid()
        .env("BRAID_SYNC_URL", DEAD_SERVER)
        .args(["create", "straggler strand"])
        .assert()
        .success();

    // C reconnects and adopts: the straggler is detected and preserved.
    let out = c.braid().args(["rotate", "--adopt"]).assert().success();
    let combined = format!(
        "{}{}",
        String::from_utf8(out.get_output().stdout.clone()).unwrap(),
        String::from_utf8(out.get_output().stderr.clone()).unwrap()
    );
    assert!(combined.contains("straggler"), "{combined}");

    let stragglers_file = c.work.join(".braid-stragglers.jsonl");
    assert!(stragglers_file.exists(), "straggler JSONL must be written");
    let content = std::fs::read_to_string(&stragglers_file).unwrap();
    assert!(content.contains("straggler strand"));

    // The adopted clone is on the new skein (1 strand) and can recover the
    // straggler by importing the file.
    assert_eq!(c.list_titles(), ["pre-rotation strand"]);
    c.braid().args(["import", stragglers_file.to_str().unwrap()]).assert().success();
    let mut titles = c.list_titles();
    titles.sort();
    assert_eq!(titles, ["pre-rotation strand", "straggler strand"]);

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rotation_requires_the_server() {
    let tmp = tempfile::tempdir().unwrap();

    let a = Clone_::new(tmp.path(), "a");
    a.braid()
        .env("BRAID_SYNC_TIMEOUT", "1")
        .args(["init", "--name", "off", "--sync-server", DEAD_SERVER])
        .assert()
        .success();
    let old_id = a.doc_id();
    a.braid().env("BRAID_SYNC_TIMEOUT", "1").args(["create", "x"]).assert().success();

    a.braid()
        .env("BRAID_SYNC_TIMEOUT", "1")
        .arg("rotate")
        .assert()
        .failure()
        .stderr(predicate::str::contains("server"));

    assert_eq!(a.doc_id(), old_id, "offline rotation must change nothing");
}
