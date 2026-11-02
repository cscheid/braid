//! `braid list | head` must die quietly on SIGPIPE like any Unix tool, not
//! panic with "failed printing to stdout: Broken pipe"
//! (plan: claude-notes/plans/2026/11/02/sigpipe.md).
//!
//! Only an allowlist of read-only commands restores SIG_DFL; everything
//! else keeps Rust's default (SIGPIPE ignored). To avoid racing the child's
//! first write, the pipe's read end is closed *before* spawning, so every
//! stdout write gets EPIPE.

#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const DEAD_SERVER: &str = "tcp://127.0.0.1:1";
const SIGPIPE: i32 = 13;

fn braid(cwd: &Path, home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_braid"));
    c.current_dir(cwd)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env("HOME", home)
        .env("BRAID_SYNC_TIMEOUT", "0.3");
    c
}

/// Run `braid <args>` with stdout wired to a pipe whose read end is closed.
fn run_closed_stdout(cwd: &Path, home: &Path, args: &[&str]) -> Output {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut c = braid(cwd, home);
    c.args(args).stdout(writer).stderr(Stdio::piped());
    // Command holds the write end; don't let the parent keep a copy open
    // beyond the spawn (irrelevant to EPIPE, but tidy).
    c.output().unwrap()
}

/// Returns (tempdir, work dir, home dir, id of one strand).
fn setup() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf, String) {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("work");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let ok = braid(&work, &home)
        .args(["init", "--name", "sp", "--sync-server", DEAD_SERVER])
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let out = braid(&work, &home).args(["create", "a strand"]).output().unwrap();
    assert!(out.status.success());
    let id = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert!(id.starts_with("br-"), "got {id:?}");
    (tmp, work, home, id)
}

fn assert_quiet_sigpipe(out: &Output, what: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked") && !stderr.contains("Broken pipe"),
        "{what}: no panic expected, stderr:\n{stderr}"
    );
    assert!(
        out.status.signal() == Some(SIGPIPE) || out.status.success(),
        "{what}: expected SIGPIPE termination or success, got {:?}",
        out.status
    );
}

#[test]
fn allowlisted_read_commands_die_quietly_on_closed_stdout() {
    let (_tmp, work, home, id) = setup();
    let cases: Vec<Vec<&str>> = vec![
        vec!["list"],
        vec!["show", &id, "--json"],
        vec!["export"],
        vec!["agents-info"],
        vec!["config"],
    ];
    for args in cases {
        let out = run_closed_stdout(&work, &home, &args);
        assert_quiet_sigpipe(&out, &args.join(" "));
    }
}

#[test]
fn non_allowlisted_commands_keep_rusts_default() {
    // `create` is mutating: it must not get SIG_DFL, so a closed stdout is
    // surfaced as an ordinary failure (panic/error), never a signal kill.
    let (_tmp, work, home, _id) = setup();
    let out = run_closed_stdout(&work, &home, &["create", "another strand"]);
    assert_ne!(
        out.status.signal(),
        Some(SIGPIPE),
        "create must not be killed by SIGPIPE: {:?}",
        out.status
    );
}
