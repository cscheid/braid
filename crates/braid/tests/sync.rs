//! End-to-end sync tests: real `braid` binary invocations talking to an
//! in-process samod sync server over TCP loopback.
//!
//! The server is a plain samod Repo with in-memory storage fed by a
//! TcpListener accept loop — the same role wss://sync.automerge.org or a
//! local relay plays in production (design decision D2).

use std::path::{Path, PathBuf};

use predicates::prelude::*;
use samod::storage::InMemoryStorage;

/// An in-process sync server. Dropping it shuts the accept loop down.
struct TestServer {
    url: String,
    repo: samod::Repo,
    accept_task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    async fn start() -> TestServer {
        let repo = samod::Repo::build_tokio()
            .with_storage(InMemoryStorage::new())
            .load()
            .await;
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

/// A simulated machine: its own HOME (and therefore its own braid cache)
/// and its own working directory.
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
            // keep tests snappy; everything here is loopback
            .env("BRAID_SYNC_TIMEOUT", "10");
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

fn create_issue(clone: &Clone_, args: &[&str]) -> String {
    let out = clone.braid().arg("create").args(args).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap().trim().to_string()
}

fn list_json(clone: &Clone_) -> serde_json::Value {
    let out = clone.braid().args(["list", "--json"]).assert().success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn fresh_clone_fetches_tracker_from_server() {
    let tmp = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;

    // Machine A: init against the test server, create an issue.
    let a = Clone_::new(tmp.path(), "a");
    a.braid()
        .args(["init", "--name", "synced", "--sync-server", &server.url])
        .assert()
        .success();
    let doc_id = a.doc_id();
    create_issue(&a, &["From machine A"]);

    // Machine B: brand-new cache, only the secret. list must fetch the
    // document from the server.
    let b = Clone_::new(tmp.path(), "b");
    b.write_secret(&doc_id, &server.url);
    b.braid()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("From machine A"));

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_clones_converge_through_server() {
    let tmp = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;

    let a = Clone_::new(tmp.path(), "a");
    a.braid()
        .args(["init", "--name", "synced", "--sync-server", &server.url])
        .assert()
        .success();
    let doc_id = a.doc_id();

    let b = Clone_::new(tmp.path(), "b");
    b.write_secret(&doc_id, &server.url);

    create_issue(&a, &["issue from A"]);
    create_issue(&b, &["issue from B"]);

    // Both clones see both issues.
    for clone in [&a, &b] {
        let issues = list_json(clone);
        let titles: Vec<&str> =
            issues.as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
        assert!(titles.contains(&"issue from A"), "missing A's issue: {titles:?}");
        assert!(titles.contains(&"issue from B"), "missing B's issue: {titles:?}");
    }

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_command_fetches_and_reports() {
    let tmp = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;

    let a = Clone_::new(tmp.path(), "a");
    a.braid()
        .args(["init", "--name", "synced", "--sync-server", &server.url])
        .assert()
        .success();
    create_issue(&a, &["one"]);
    create_issue(&a, &["two"]);

    let b = Clone_::new(tmp.path(), "b");
    b.write_secret(&a.doc_id(), &server.url);
    b.braid()
        .arg("sync")
        .assert()
        .success()
        .stdout(predicate::str::contains("2 issue"));

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_falls_back_to_cache_with_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;

    let a = Clone_::new(tmp.path(), "a");
    a.braid()
        .args(["init", "--name", "synced", "--sync-server", &server.url])
        .assert()
        .success();
    create_issue(&a, &["created online"]);

    // Take the server away and point at a dead port.
    let doc_id = a.doc_id();
    server.stop().await;
    a.write_secret(&doc_id, "tcp://127.0.0.1:1");

    // Reads warn but work from cache.
    a.braid()
        .env("BRAID_SYNC_TIMEOUT", "1")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("created online"))
        .stderr(predicate::str::contains("offline"));

    // Writes work offline too.
    a.braid().env("BRAID_SYNC_TIMEOUT", "1").args(["create", "created offline"]).assert().success();
    a.braid()
        .env("BRAID_SYNC_TIMEOUT", "1")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("created offline"));
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_command_fails_when_offline() {
    let tmp = tempfile::tempdir().unwrap();

    let a = Clone_::new(tmp.path(), "a");
    // init offline against a dead server: should still succeed locally...
    a.braid()
        .env("BRAID_SYNC_TIMEOUT", "1")
        .args(["init", "--name", "lonely", "--sync-server", "tcp://127.0.0.1:1"])
        .assert()
        .success();

    // ...but explicit `braid sync` must fail loudly.
    a.braid()
        .env("BRAID_SYNC_TIMEOUT", "1")
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicate::str::contains("offline").or(predicate::str::contains("could not")));
}

#[tokio::test(flavor = "multi_thread")]
async fn no_cache_mode_is_stateless() {
    let tmp = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;

    let a = Clone_::new(tmp.path(), "a");
    a.braid()
        .args(["init", "--name", "synced", "--sync-server", &server.url])
        .assert()
        .success();
    create_issue(&a, &["persistent issue"]);

    // BRAID_NO_CACHE + live server: everything is fetched fresh and works.
    a.braid()
        .env("BRAID_NO_CACHE", "1")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("persistent issue"));

    // BRAID_NO_CACHE + dead server: nothing local to fall back on, even
    // though the cache on disk has the doc — proves the mode is stateless.
    let doc_id = a.doc_id();
    server.stop().await;
    a.write_secret(&doc_id, "tcp://127.0.0.1:1");
    a.braid()
        .env("BRAID_NO_CACHE", "1")
        .env("BRAID_SYNC_TIMEOUT", "1")
        .arg("list")
        .assert()
        .failure();

    // ...while the cached mode still works offline.
    a.braid()
        .env("BRAID_SYNC_TIMEOUT", "1")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("persistent issue"));
}

#[tokio::test(flavor = "multi_thread")]
async fn doc_created_offline_announces_on_first_sync() {
    // D13: init works offline; the doc reaches the server on the first
    // successful sync afterwards.
    let tmp = tempfile::tempdir().unwrap();

    let a = Clone_::new(tmp.path(), "a");
    a.braid()
        .env("BRAID_SYNC_TIMEOUT", "1")
        .args(["init", "--name", "late-announce", "--sync-server", "tcp://127.0.0.1:1"])
        .assert()
        .success();
    let doc_id = a.doc_id();
    a.braid()
        .env("BRAID_SYNC_TIMEOUT", "1")
        .args(["create", "made offline"])
        .assert()
        .success();

    // Server comes up; repoint A at it and sync.
    let server = TestServer::start().await;
    a.write_secret(&doc_id, &server.url);
    a.braid().arg("sync").assert().success();

    // A fresh clone can now fetch everything from the server.
    let b = Clone_::new(tmp.path(), "b");
    b.write_secret(&doc_id, &server.url);
    b.braid()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("made offline"));

    server.stop().await;
}
