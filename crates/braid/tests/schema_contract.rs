//! Contract tests: `braid export` output must conform to
//! docs/schemas/strand.schema.json (strand br-jsonl-schema).
//!
//! These tests validate *real* export output — built through the full CLI
//! (create/update/comment/dep/close and a beads-format import) — so any
//! drift between the implementation and the published contract fails here.

use std::path::PathBuf;

use predicates::prelude::*;

const DEAD_SERVER: &str = "tcp://127.0.0.1:1";

fn schema() -> jsonschema::Validator {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/schemas/strand.schema.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let json: serde_json::Value = serde_json::from_str(&text).expect("schema is valid JSON");
    jsonschema::validator_for(&json).expect("schema is a valid 2020-12 JSON Schema")
}

fn assert_conforms(validator: &jsonschema::Validator, line: &str) {
    let record: serde_json::Value = serde_json::from_str(line).expect("export line is JSON");
    let errors: Vec<String> = validator
        .iter_errors(&record)
        .map(|e| format!("{e} (at {})", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "export line violates contract:\n{line}\nerrors:\n{}", errors.join("\n"));
}

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
            .args(["init", "--name", "contract", "--sync-server", DEAD_SERVER])
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

    fn export(&self) -> String {
        let out = self.braid().arg("export").assert().success();
        String::from_utf8(out.get_output().stdout.clone()).unwrap()
    }
}

#[test]
fn cli_built_skein_exports_conforming_records() {
    let (_tmp, t) = Skein::new();
    let validator = schema();

    // a minimal strand
    t.create(&["Minimal strand"]);

    // a fully loaded strand
    let full = t.create(&[
        "Full strand",
        "--description",
        "Body with\nnewlines and unicode: héllo → 🎉",
        "--type",
        "feature",
        "--priority",
        "0",
        "--label",
        "alpha",
        "--label",
        "beta",
        "--slug",
        "full-strand",
        "--assignee",
        "agent-7",
    ]);
    t.braid()
        .args(["update", &full, "--design", "design notes", "--acceptance-criteria=- works", "--notes", "misc", "--external-ref", "https://example.com/x"])
        .assert()
        .success();
    t.braid().args(["comment", &full, "a comment"]).assert().success();
    let other = t.create(&["Dep target"]);
    t.braid().args(["dep", "add", &full, &other, "--type", "waits-for"]).assert().success();
    // a dangling edge is legal per the contract
    let ghost = t.create(&["Ghostly"]);
    t.braid().args(["dep", "add", &full, &ghost, "--type", "related"]).assert().success();

    // a closed strand with a reason
    let closed = t.create(&["Will close"]);
    t.braid().args(["close", &closed, "--reason", "done"]).assert().success();

    // deferred strands: with a wake time, and dateless
    let dated = t.create(&["Deferred with date"]);
    t.braid().args(["defer", &dated, "--until", "2099-07-01"]).assert().success();
    let dateless = t.create(&["Deferred without date"]);
    t.braid().args(["defer", &dateless]).assert().success();

    let export = t.export();
    let lines: Vec<&str> = export.lines().collect();
    assert_eq!(lines.len(), 7);
    for line in lines {
        assert_conforms(&validator, line);
    }
}

#[test]
fn beads_import_exports_conforming_records() {
    let (_tmp, t) = Skein::new();
    let validator = schema();

    let beads = concat!(
        r#"{"id":"bd-0gsj","title":"Beads bug","description":"desc","status":"closed","priority":1,"issue_type":"bug","created_at":"2026-04-27T14:26:53.718601600Z","created_by":"cderv","updated_at":"2026-05-04T13:45:00.482193500Z","closed_at":"2026-05-04T13:45:00.481818100Z","close_reason":"merged","external_ref":"https://example.com/138","source_repo":".","compaction_level":0,"labels":["a","b"],"dependencies":[{"issue_id":"bd-0gsj","depends_on_id":"bd-ntnx","type":"discovered-from","created_at":"2026-04-27T14:26:53Z","created_by":"cderv","metadata":"{}","thread_id":""}],"comments":[{"id":15,"issue_id":"bd-0gsj","author":"cderv","text":"hello","created_at":"2026-04-28T15:37:36Z"}]}"#,
        "\n",
        r#"{"id":"bd-0a3b","title":"Completed chore","status":"completed","priority":3,"issue_type":"chore","created_at":"2026-05-04T18:15:54.920222Z","created_by":"cscheid","updated_at":"2026-05-04T19:06:15.700658Z"}"#,
        "\n",
    );
    let file = t.work.join("issues.jsonl");
    std::fs::write(&file, beads).unwrap();
    t.braid().args(["import", file.to_str().unwrap()]).assert().success();

    for line in t.export().lines() {
        assert_conforms(&validator, line);
    }
}

#[test]
fn import_rejects_ids_the_contract_forbids() {
    // anything import accepts, export will emit — so import must enforce
    // the schema's id constraints (no colons, no whitespace)
    let (_tmp, t) = Skein::new();
    for bad_id in ["has:colon", "has space"] {
        let file = t.work.join("bad.jsonl");
        std::fs::write(
            &file,
            format!(r#"{{"id":"{bad_id}","title":"t","status":"open"}}"#) + "\n",
        )
        .unwrap();
        t.braid()
            .args(["import", file.to_str().unwrap()])
            .assert()
            .failure()
            .stderr(predicate::str::contains("id"));
    }
}

/// The schema itself must reject malformed records (these never come from
/// braid; they guard downstream validators against schema bugs).
#[test]
fn schema_rejects_malformed_records() {
    let validator = schema();
    let valid = serde_json::json!({
        "id": "br-ok",
        "title": "t",
        "status": "open",
        "priority": 2,
        "issue_type": "task",
        "created_at": "2026-06-04T00:00:00.000000Z",
        "created_by": "x",
        "updated_at": "2026-06-04T00:00:00.000000Z",
    });
    assert!(validator.is_valid(&valid), "baseline record must validate");

    let mutate = |f: &dyn Fn(&mut serde_json::Value)| {
        let mut v = valid.clone();
        f(&mut v);
        v
    };

    let cases: Vec<(&str, serde_json::Value)> = vec![
        ("missing title", mutate(&|v| {
            v.as_object_mut().unwrap().remove("title");
        })),
        ("colon in id", mutate(&|v| v["id"] = "br:bad".into())),
        ("non-integer priority", mutate(&|v| v["priority"] = "high".into())),
        ("non-string defer_until", mutate(&|v| v["defer_until"] = 42.into())),
        ("unknown top-level field", mutate(&|v| v["surprise"] = true.into())),
        ("empty labels array", mutate(&|v| v["labels"] = serde_json::json!([]))),
        ("duplicate labels", mutate(&|v| v["labels"] = serde_json::json!(["a", "a"]))),
        ("dependency missing type", mutate(&|v| {
            v["dependencies"] = serde_json::json!({
                "br-x:blocks": {"depends_on_id": "br-x", "created_at": "2026-06-04T00:00:00Z", "created_by": "x"}
            })
        })),
        ("dependency key without colon", mutate(&|v| {
            v["dependencies"] = serde_json::json!({
                "br-x": {"depends_on_id": "br-x", "type": "blocks", "created_at": "2026-06-04T00:00:00Z", "created_by": "x"}
            })
        })),
        ("comment missing text", mutate(&|v| {
            v["comments"] = serde_json::json!({
                "c-1": {"id": "c-1", "author": "x", "created_at": "2026-06-04T00:00:00Z"}
            })
        })),
    ];
    for (name, record) in cases {
        assert!(!validator.is_valid(&record), "schema should reject: {name}\n{record}");
    }
}
