//! e2e tests for `braid agents-info` and its `--install` skill installer.
//! No skein required — installing is a purely local file operation.

use predicates::prelude::*;

fn braid() -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("braid").unwrap();
    c.env_clear().env("PATH", std::env::var("PATH").unwrap());
    c
}

#[test]
fn agents_info_prints_the_guide() {
    braid()
        .arg("agents-info")
        .assert()
        .success()
        .stdout(predicate::str::contains("braid agents-info"));
}

#[test]
fn install_writes_a_valid_skill_with_frontmatter() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nested").join("braid"); // parent dirs created
    braid().args(["agents-info", "--install", dir.to_str().unwrap()]).assert().success();

    let skill = std::fs::read_to_string(dir.join("SKILL.md")).unwrap();
    // YAML frontmatter must be the very first bytes for the skill to load
    assert!(skill.starts_with("---\n"), "frontmatter leads the file: {skill}");
    // name derived from the directory; description present for auto-invocation
    assert!(skill.contains("name: braid"), "skill name from dir basename");
    assert!(skill.contains("description: braid issue tracking"));
    // body in a managed block, deferring to the authoritative guide
    assert!(skill.contains("<!-- BEGIN BRAID"));
    assert!(skill.contains("<!-- END BRAID -->"));
    assert!(skill.contains("braid agents-info"));
}

#[test]
fn reinstall_does_not_duplicate_and_preserves_trailing_content() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let path = dir.join("SKILL.md");

    // first install into an empty dir
    braid().args(["agents-info", "--install", dir.to_str().unwrap()]).assert().success();

    // append user content *after* the braid block, then re-install
    let after_first = std::fs::read_to_string(&path).unwrap();
    assert!(after_first.starts_with("---\n"), "frontmatter at top");
    std::fs::write(&path, format!("{after_first}\n## Project notes\nhand-written\n")).unwrap();

    braid().args(["agents-info", "--install", dir.to_str().unwrap()]).assert().success();
    let after_second = std::fs::read_to_string(&path).unwrap();

    // exactly one block, still valid frontmatter, trailing user content kept
    assert!(after_second.starts_with("---\n"));
    assert_eq!(after_second.matches("<!-- BEGIN BRAID").count(), 1, "no duplication");
    assert!(after_second.contains("## Project notes"));
    assert!(after_second.contains("hand-written"));
}

#[test]
fn install_refuses_to_clobber_a_non_braid_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let path = dir.join("SKILL.md");
    std::fs::write(&path, "# Someone else's skill\n\nimportant\n").unwrap();

    braid()
        .args(["agents-info", "--install", dir.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not written by braid"));

    // the foreign file is left untouched
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Someone else's skill\n\nimportant\n");
}
