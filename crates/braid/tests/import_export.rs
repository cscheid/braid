//! e2e tests for `braid import` (beads and braid JSONL) and `braid export`.

use std::path::PathBuf;

use predicates::prelude::*;

const DEAD_SERVER: &str = "tcp://127.0.0.1:1";

struct Skein {
    home: PathBuf,
    work: PathBuf,
}

impl Skein {
    fn new_at(tmp: &std::path::Path, name: &str) -> Skein {
        let home = tmp.join(format!("{name}-home"));
        let work = tmp.join(format!("{name}-work"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        let t = Skein { home, work };
        t.braid().args(["init", "--name", name, "--sync-server", DEAD_SERVER]).assert().success();
        t
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

    fn show_json(&self, id: &str) -> serde_json::Value {
        let out = self.braid().args(["show", id, "--json"]).assert().success();
        serde_json::from_slice(&out.get_output().stdout).unwrap()
    }
}

/// Two realistic beads JSONL lines, abridged from the real example data:
/// one full-featured closed bug (labels, deps, comments with integer ids,
/// close_reason, external_ref, beads-only fields) and one open chore.
/// Note the beads-only fields (source_repo, compaction_level, …) and the
/// "completed" status alias.
const BEADS_JSONL: &str = concat!(
    r#"{"id":"bd-0gsj","title":"Fix CRLF handling in tree-sitter grammar","description":"Grammar parses pipe tables incorrectly with CRLF line endings.","design":"Reproducer: two layers.","acceptance_criteria":"Parse trees identical for CRLF and LF.","status":"closed","priority":1,"issue_type":"bug","created_at":"2026-04-27T14:26:53.718601600Z","created_by":"cderv","updated_at":"2026-05-04T13:45:00.482193500Z","closed_at":"2026-05-04T13:45:00.481818100Z","close_reason":"PR #139 merged","external_ref":"https://github.com/quarto-dev/q2/issues/138","source_repo":".","compaction_level":0,"original_size":0,"labels":["tree-sitter","windows"],"dependencies":[{"issue_id":"bd-0gsj","depends_on_id":"bd-ntnx","type":"discovered-from","created_at":"2026-04-27T14:26:53.718601600Z","created_by":"cderv","metadata":"{}","thread_id":""}],"comments":[{"id":15,"issue_id":"bd-0gsj","author":"cderv","text":"PR #139 opened.","created_at":"2026-04-28T15:37:36Z"},{"id":16,"issue_id":"bd-0gsj","author":"cderv","text":"Verification tomorrow.","created_at":"2026-04-28T16:05:51Z"}]}"#,
    "\n",
    r#"{"id":"bd-0a3b","title":"Cargo: upgrade rand","description":"Major upgrade surfaced by survey.","status":"completed","priority":3,"issue_type":"chore","created_at":"2026-05-04T18:15:54.920222Z","created_by":"cscheid","updated_at":"2026-05-04T19:06:15.700658Z","source_repo":".","compaction_level":0,"original_size":0,"labels":["cargo","deps"]}"#,
    "\n",
);

#[test]
fn import_beads_jsonl_maps_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let t = Skein::new_at(tmp.path(), "imp");
    let jsonl = t.work.join("issues.jsonl");
    std::fs::write(&jsonl, BEADS_JSONL).unwrap();

    t.braid()
        .args(["import", jsonl.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("2"));

    // full-featured issue
    let bug = t.show_json("bd-0gsj");
    assert_eq!(bug["title"], "Fix CRLF handling in tree-sitter grammar");
    assert_eq!(bug["status"], "closed");
    assert_eq!(bug["issue_type"], "bug");
    assert_eq!(bug["priority"], 1);
    assert_eq!(bug["created_by"], "cderv");
    assert_eq!(bug["close_reason"], "PR #139 merged");
    assert_eq!(bug["external_ref"], "https://github.com/quarto-dev/q2/issues/138");
    assert_eq!(bug["labels"], serde_json::json!(["tree-sitter", "windows"]));
    assert_eq!(bug["design"], "Reproducer: two layers.");

    // dependency became a keyed map entry; dangling target tolerated
    let deps = bug["dependencies"].as_object().unwrap();
    assert_eq!(deps.len(), 1);
    let dep = &deps["bd-ntnx:discovered-from"];
    assert_eq!(dep["depends_on_id"], "bd-ntnx");
    assert_eq!(dep["type"], "discovered-from");

    // integer comment ids became fresh c- ids; content preserved
    let comments = bug["comments"].as_object().unwrap();
    assert_eq!(comments.len(), 2);
    for (key, c) in comments {
        assert!(key.starts_with("c-"), "comment key {key:?} should be a c- id");
        assert_eq!(c["id"], key.as_str());
        assert_eq!(c["author"], "cderv");
    }
    let texts: Vec<&str> = comments.values().map(|c| c["text"].as_str().unwrap()).collect();
    assert!(texts.contains(&"PR #139 opened."));

    // "completed" status alias maps to closed
    let chore = t.show_json("bd-0a3b");
    assert_eq!(chore["status"], "closed");
}

#[test]
fn import_is_an_upsert() {
    let tmp = tempfile::tempdir().unwrap();
    let t = Skein::new_at(tmp.path(), "imp");
    let jsonl = t.work.join("issues.jsonl");
    std::fs::write(&jsonl, BEADS_JSONL).unwrap();

    t.braid().args(["import", jsonl.to_str().unwrap()]).assert().success();
    // touch one issue locally, then re-import: the file's state wins
    t.braid().args(["update", "bd-0a3b", "--title", "locally changed"]).assert().success();
    t.braid().args(["import", jsonl.to_str().unwrap()]).assert().success();
    assert_eq!(t.show_json("bd-0a3b")["title"], "Cargo: upgrade rand");

    // and nothing got duplicated
    let out = t.braid().args(["list", "--json"]).assert().success();
    let all: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 2);
}

#[test]
fn import_rejects_malformed_lines_atomically() {
    let tmp = tempfile::tempdir().unwrap();
    let t = Skein::new_at(tmp.path(), "imp");
    let jsonl = t.work.join("bad.jsonl");
    std::fs::write(&jsonl, format!("{BEADS_JSONL}this is not json\n")).unwrap();

    t.braid()
        .args(["import", jsonl.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("line 3"));

    // nothing was imported (atomic failure)
    let out = t.braid().args(["list", "--json"]).assert().success();
    let all: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 0);
}

#[test]
fn import_preserves_defer_until() {
    let tmp = tempfile::tempdir().unwrap();
    let t = Skein::new_at(tmp.path(), "imp");
    let jsonl = t.work.join("deferred.jsonl");
    // a braid-format line and a beads-style line (beads-only fields in
    // tow), both carrying defer_until
    std::fs::write(
        &jsonl,
        concat!(
            r#"{"id":"br-park1","title":"Braid deferred","status":"deferred","priority":2,"issue_type":"task","created_at":"2026-06-01T00:00:00.000000Z","created_by":"t","updated_at":"2026-06-01T00:00:00.000000Z","defer_until":"2026-09-01T00:00:00.000000Z"}"#,
            "\n",
            r#"{"id":"bd-park2","title":"Beads deferred","status":"deferred","priority":2,"issue_type":"task","created_at":"2026-06-01T00:00:00Z","created_by":"t","updated_at":"2026-06-01T00:00:00Z","defer_until":"2026-09-15T12:00:00Z","source_repo":".","compaction_level":0}"#,
            "\n",
        ),
    )
    .unwrap();

    t.braid().args(["import", jsonl.to_str().unwrap()]).assert().success();
    assert_eq!(t.show_json("br-park1")["defer_until"], "2026-09-01T00:00:00.000000Z");
    // imported timestamps are preserved as-is, not normalized
    assert_eq!(t.show_json("bd-park2")["defer_until"], "2026-09-15T12:00:00Z");

    // and export carries the field back out
    let out = t.braid().arg("export").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains(r#""defer_until":"2026-09-01T00:00:00.000000Z""#));
}

#[test]
fn export_emits_jsonl_sorted_by_id() {
    let tmp = tempfile::tempdir().unwrap();
    let t = Skein::new_at(tmp.path(), "exp");
    let jsonl = t.work.join("issues.jsonl");
    std::fs::write(&jsonl, BEADS_JSONL).unwrap();
    t.braid().args(["import", jsonl.to_str().unwrap()]).assert().success();

    let out = t.braid().arg("export").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(first["id"], "bd-0a3b");
    assert_eq!(second["id"], "bd-0gsj");
}

#[test]
fn export_import_round_trips_exactly() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Skein::new_at(tmp.path(), "a");
    let jsonl = a.work.join("issues.jsonl");
    std::fs::write(&jsonl, BEADS_JSONL).unwrap();
    a.braid().args(["import", jsonl.to_str().unwrap()]).assert().success();

    let exported = a.braid().arg("export").assert().success();
    let exported = String::from_utf8(exported.get_output().stdout.clone()).unwrap();

    // import A's export into a brand-new skein B
    let b = Skein::new_at(tmp.path(), "b");
    let exp_file = b.work.join("export.jsonl");
    std::fs::write(&exp_file, &exported).unwrap();
    b.braid().args(["import", exp_file.to_str().unwrap()]).assert().success();

    // B's export must equal A's export byte-for-byte (braid-format import
    // preserves comment ids, keys, everything)
    let again = b.braid().arg("export").assert().success();
    let again = String::from_utf8(again.get_output().stdout.clone()).unwrap();
    assert_eq!(exported, again);
}
