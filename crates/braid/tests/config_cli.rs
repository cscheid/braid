//! e2e tests for `braid config` (strand br-y6ra26a3): a diagnostic that
//! prints every resolved config field, its value, and where it came from,
//! so users can debug which layer (env / .braid.toml / user config) won.
//! The doc id stays a bearer secret here — only a redacted prefix is shown.

use std::path::PathBuf;

const DEAD_SERVER: &str = "tcp://127.0.0.1:1";

struct Env {
    home: PathBuf,
    work: PathBuf,
}

impl Env {
    fn new() -> (tempfile::TempDir, Env) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        (tmp, Env { home, work })
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
            .args(["init", "--name", "cfg", "--sync-server", DEAD_SERVER])
            .assert()
            .success();
        let secret = std::fs::read_to_string(self.work.join(".braid.toml")).unwrap();
        let parsed: toml::Value = toml::from_str(&secret).unwrap();
        parsed["doc_id"].as_str().unwrap().to_string()
    }
}

#[test]
fn config_shows_each_field_and_its_source() {
    let (_tmp, t) = Env::new();
    t.init();

    let out = t.braid().arg("config").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();

    // every resolved field is named
    for field in ["doc_id", "sync_server", "author"] {
        assert!(stdout.contains(field), "config should list {field}:\n{stdout}");
    }
    // the doc id resolved from the .braid.toml init wrote
    assert!(stdout.contains(".braid.toml"), "config should name the source file:\n{stdout}");
    // the sync server value is shown
    assert!(stdout.contains(DEAD_SERVER), "config should show the sync server:\n{stdout}");
}

#[test]
fn config_redacts_the_doc_id() {
    // `braid config` is a safe-to-run diagnostic: it must NOT leak the
    // bearer doc id (that is `braid secret`'s job), only a redacted prefix.
    let (_tmp, t) = Env::new();
    let doc_id = t.init();

    let out = t.braid().arg("config").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        !stdout.contains(&doc_id) && !stderr.contains(&doc_id),
        "config must not print the full doc id:\n{stdout}\n{stderr}"
    );
    let prefix: String = doc_id.chars().take(6).collect();
    assert!(
        stdout.contains(&prefix),
        "config should show a redacted prefix ({prefix}…):\n{stdout}"
    );
}

#[test]
fn config_reports_user_config_provenance() {
    // No .braid.toml: a .braid-project marker selects a project in the
    // user-level config. `braid config` should point at that file + project.
    let (_tmp, t) = Env::new();
    let cfg_dir = t.home.join(".config/braid");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(
        cfg_dir.join("projects.toml"),
        format!(
            "[projects.demo]\ndoc_id = \"aaaaaaaaaaaaaaaa\"\nsync_server = \"{DEAD_SERVER}\"\n"
        ),
    )
    .unwrap();
    std::fs::write(t.work.join(".braid-project"), "demo\n").unwrap();

    let out = t.braid().arg("config").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("projects.toml"), "should name the user config file:\n{stdout}");
    assert!(stdout.contains("demo"), "should name the selected project:\n{stdout}");
}
