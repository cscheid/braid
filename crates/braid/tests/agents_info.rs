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
fn install_writes_skill_with_managed_block() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nested").join("braid"); // parent dirs created
    braid().args(["agents-info", "--install", dir.to_str().unwrap()]).assert().success();

    let skill = std::fs::read_to_string(dir.join("SKILL.md")).unwrap();
    assert!(skill.contains("<!-- BEGIN BRAID"), "managed block present: {skill}");
    assert!(skill.contains("<!-- END BRAID -->"));
    assert!(skill.contains("braid agents-info"), "defers to the authoritative guide");
}

#[test]
fn reinstall_does_not_duplicate_and_preserves_user_content() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let path = dir.join("SKILL.md");

    // a pre-existing file with user content and no braid block
    std::fs::write(&path, "# My own notes\n\nkeep me\n").unwrap();
    braid().args(["agents-info", "--install", dir.to_str().unwrap()]).assert().success();

    // append a trailing line *after* the block, then re-install
    let after_first = std::fs::read_to_string(&path).unwrap();
    assert!(after_first.starts_with("# My own notes\n\nkeep me\n"));
    std::fs::write(&path, format!("{after_first}\n## trailing user section\nhand-written\n"))
        .unwrap();

    braid().args(["agents-info", "--install", dir.to_str().unwrap()]).assert().success();
    let after_second = std::fs::read_to_string(&path).unwrap();

    // exactly one managed block; both user sections preserved
    assert_eq!(after_second.matches("<!-- BEGIN BRAID").count(), 1, "no duplication");
    assert!(after_second.contains("# My own notes"));
    assert!(after_second.contains("keep me"));
    assert!(after_second.contains("## trailing user section"));
    assert!(after_second.contains("hand-written"));
}
