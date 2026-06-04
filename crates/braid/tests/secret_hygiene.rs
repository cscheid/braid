//! e2e tests for doc-id hygiene (strand br-redact-doc-id): the doc id is a
//! bearer capability and must never appear in ordinary output — errors and
//! status lines show a redacted prefix; full disclosure is an explicit act
//! via `braid secret`.

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
        (tmp, Skein { home, work })
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

    fn init(&self) -> String {
        self.braid()
            .args(["init", "--name", "hygiene", "--sync-server", DEAD_SERVER])
            .assert()
            .success();
        let secret = std::fs::read_to_string(self.work.join(".braid.toml")).unwrap();
        let parsed: toml::Value = toml::from_str(&secret).unwrap();
        parsed["doc_id"].as_str().unwrap().to_string()
    }
}

#[test]
fn init_does_not_print_the_doc_id() {
    let (_tmp, t) = Skein::new();
    let out = t
        .braid()
        .args(["init", "--name", "hygiene", "--sync-server", DEAD_SERVER])
        .assert()
        .success();
    let doc_id = {
        let secret = std::fs::read_to_string(t.work.join(".braid.toml")).unwrap();
        let parsed: toml::Value = toml::from_str(&secret).unwrap();
        parsed["doc_id"].as_str().unwrap().to_string()
    };
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        !stdout.contains(&doc_id) && !stderr.contains(&doc_id),
        "init must not print the full doc id (it goes to logs/transcripts):\n{stdout}\n{stderr}"
    );
    // ...but it should still say where the secret lives and how to see it
    assert!(stdout.contains(".braid.toml"), "init should name the secret file:\n{stdout}");
    assert!(stdout.contains("braid secret"), "init should point at braid secret:\n{stdout}");
}

#[test]
fn errors_redact_the_doc_id() {
    let (_tmp, t) = Skein::new();
    // valid-format doc id that is in no cache, against a dead server
    let foreign = {
        let other = Skein::new();
        other.1.init()
    };
    std::fs::write(
        t.work.join(".braid.toml"),
        format!("doc_id = \"{foreign}\"\nsync_server = \"{DEAD_SERVER}\"\n"),
    )
    .unwrap();

    let out = t.braid().arg("list").assert().failure();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        !stderr.contains(&foreign),
        "error output must not contain the full doc id:\n{stderr}"
    );
    let prefix: String = foreign.chars().take(6).collect();
    assert!(
        stderr.contains(&prefix),
        "error should include a redacted prefix ({prefix}…) for disambiguation:\n{stderr}"
    );
}

#[test]
fn braid_secret_is_the_explicit_disclosure_path() {
    let (_tmp, t) = Skein::new();
    let doc_id = t.init();

    let out = t.braid().arg("secret").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains(&doc_id), "braid secret must print the full doc id:\n{stdout}");
    assert!(
        stdout.contains(DEAD_SERVER),
        "braid secret should print the sync server too (paste-ready):\n{stdout}"
    );
    // paste-ready: the output parses as the .braid.toml shape
    let parsed: toml::Value = toml::from_str(&stdout).expect("braid secret output is valid TOML");
    assert_eq!(parsed["doc_id"].as_str().unwrap(), doc_id);
}

#[test]
fn braid_secret_warns_on_stderr() {
    // the warning must be on stderr so stdout stays cleanly pasteable
    let (_tmp, t) = Skein::new();
    t.init();
    t.braid()
        .arg("secret")
        .assert()
        .success()
        .stderr(predicate::str::contains("read/write"));
}

#[test]
fn agents_info_documents_the_secret_command() {
    let (_tmp, t) = Skein::new();
    let out = t.braid().arg("agents-info").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("braid secret"), "agents-info should document braid secret");
}
