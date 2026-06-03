//! e2e tests for human-readable listing output (strand br-list-format).
//!
//! Contract: columns are width-aligned across rows regardless of id
//! length; piped (non-TTY) output has no header or footer rows, so
//! `braid list | wc -l` counts strands and grep/awk see only data rows.
//! (Header + count footer appear only on a TTY, which e2e tests can't
//! exercise; alignment applies everywhere.)

use std::path::PathBuf;

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
        t.braid()
            .args(["init", "--name", "fmt", "--sync-server", DEAD_SERVER])
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

    fn stdout(&self, args: &[&str]) -> String {
        let out = self.braid().args(args).assert().success();
        String::from_utf8(out.get_output().stdout.clone()).unwrap()
    }
}

/// Column-start position of `needle` within the line that contains it.
fn col_of(haystack: &str, needle: &str) -> usize {
    let line = haystack
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle:?} in:\n{haystack}"));
    line.find(needle).unwrap()
}

#[test]
fn list_columns_align_across_mixed_id_lengths() {
    let (_tmp, t) = Skein::new();
    t.create(&["Short id strand"]);
    t.create(&["Long id strand", "--slug", "a-rather-long-slug-here", "--type", "question"]);

    let out = t.stdout(&["list"]);

    // titles start at the same column
    assert_eq!(
        col_of(&out, "Short id strand"),
        col_of(&out, "Long id strand"),
        "title columns must align:\n{out}"
    );
    // and so do the status and type columns
    assert_eq!(col_of(&out, "task"), col_of(&out, "question"), "type columns:\n{out}");
    let opens: Vec<usize> = out
        .lines()
        .filter_map(|l| l.find(" open"))
        .collect();
    assert_eq!(opens.len(), 2);
    assert_eq!(opens[0], opens[1], "status columns must align:\n{out}");
}

#[test]
fn piped_list_has_only_data_rows() {
    let (_tmp, t) = Skein::new();
    t.create(&["One"]);
    t.create(&["Two"]);

    let out = t.stdout(&["list"]);
    assert_eq!(out.lines().count(), 2, "no header/footer when piped:\n{out}");
    for line in out.lines() {
        assert!(line.starts_with("br-"), "every row starts with an id:\n{out}");
    }
}

#[test]
fn ready_and_search_share_the_aligned_format() {
    let (_tmp, t) = Skein::new();
    t.create(&["Findable apple", "--slug", "with-a-long-slug"]);
    t.create(&["Findable banana"]);

    for args in [vec!["ready"], vec!["search", "findable"]] {
        let out = t.stdout(&args);
        assert_eq!(
            col_of(&out, "Findable apple"),
            col_of(&out, "Findable banana"),
            "{args:?} titles must align:\n{out}"
        );
    }
}

#[test]
fn blocked_listing_aligns_and_names_blockers() {
    let (_tmp, t) = Skein::new();
    let blocked1 = t.create(&["Blocked one"]);
    let blocked2 = t.create(&["Blocked two with longer title", "--slug", "long-slugged-strand"]);
    let blocker = t.create(&["The blocker"]);
    t.braid().args(["dep", "add", &blocked1, &blocker]).assert().success();
    t.braid().args(["dep", "add", &blocked2, &blocker]).assert().success();

    let out = t.stdout(&["blocked"]);
    assert_eq!(out.lines().count(), 2);
    assert_eq!(
        col_of(&out, "Blocked one"),
        col_of(&out, "Blocked two"),
        "blocked titles must align:\n{out}"
    );
    for line in out.lines() {
        assert!(
            line.contains(&format!("blocked by {blocker}")),
            "each row names its blockers:\n{out}"
        );
    }
}
