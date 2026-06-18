//! e2e tests for the xtask repo-automation binary.
//!
//! `ci` itself is exercised via `--dry-run` (running the real pipeline
//! here would nest cargo-in-cargo); hook installation runs against
//! scratch git repositories.

use std::path::Path;

use predicates::prelude::*;

fn xtask() -> assert_cmd::Command {
    assert_cmd::Command::cargo_bin("xtask").unwrap()
}

/// A scratch git repository (no commits needed — hooks only).
fn scratch_repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let st = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(tmp.path())
        .status()
        .unwrap();
    assert!(st.success());
    tmp
}

fn hook_path(repo: &Path) -> std::path::PathBuf {
    repo.join(".git/hooks/pre-push")
}

// ---------------------------------------------------------------------------
// dispatch
// ---------------------------------------------------------------------------

#[test]
fn no_subcommand_prints_usage_and_fails() {
    xtask()
        .assert()
        .failure()
        .stderr(predicate::str::contains("usage").and(predicate::str::contains("install-hooks")));
}

#[test]
fn unknown_subcommand_prints_usage_and_fails() {
    xtask()
        .arg("frobnicate")
        .assert()
        .failure()
        .stderr(predicate::str::contains("frobnicate").and(predicate::str::contains("usage")));
}

// ---------------------------------------------------------------------------
// ci --dry-run
// ---------------------------------------------------------------------------

#[test]
fn ci_dry_run_prints_the_pipeline_in_order() {
    let out = xtask().args(["ci", "--dry-run"]).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        [
            "cargo fmt --all --check",
            "cargo clippy --all-targets -- -D warnings",
            "cargo build --all-targets",
            "cargo test",
            "npm --prefix ui run test",
        ],
        "ci --dry-run must print exactly the pipeline, one command per line"
    );
}

#[test]
fn ci_rejects_unknown_flags() {
    xtask()
        .args(["ci", "--frobnicate"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--frobnicate"));
}

// ---------------------------------------------------------------------------
// install-hooks
// ---------------------------------------------------------------------------

#[test]
fn install_hooks_writes_an_executable_pre_push_hook() {
    let repo = scratch_repo();
    xtask()
        .arg("install-hooks")
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("pre-push"));

    let hook = hook_path(repo.path());
    let content = std::fs::read_to_string(&hook).unwrap();
    assert!(content.starts_with("#!/bin/sh"), "hook must be a shell script:\n{content}");
    assert!(
        content.contains("cargo xtask install-hooks"),
        "hook must carry the ownership marker:\n{content}"
    );
    assert!(content.contains("cargo xtask ci"), "hook must run the ci pipeline:\n{content}");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&hook).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "hook must be executable, mode {mode:o}");
    }
}

#[test]
fn install_hooks_is_idempotent() {
    let repo = scratch_repo();
    xtask().arg("install-hooks").current_dir(repo.path()).assert().success();
    let first = std::fs::read_to_string(hook_path(repo.path())).unwrap();

    xtask().arg("install-hooks").current_dir(repo.path()).assert().success();
    let second = std::fs::read_to_string(hook_path(repo.path())).unwrap();
    assert_eq!(first, second);
}

#[test]
fn install_hooks_refuses_to_clobber_a_foreign_hook() {
    let repo = scratch_repo();
    let hook = hook_path(repo.path());
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho my precious hook\n").unwrap();

    xtask()
        .arg("install-hooks")
        .current_dir(repo.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("pre-push"));

    let content = std::fs::read_to_string(&hook).unwrap();
    assert_eq!(content, "#!/bin/sh\necho my precious hook\n", "foreign hook must be untouched");
}

#[test]
fn install_hooks_works_from_a_subdirectory() {
    let repo = scratch_repo();
    let sub = repo.path().join("deep/down");
    std::fs::create_dir_all(&sub).unwrap();

    xtask().arg("install-hooks").current_dir(&sub).assert().success();
    assert!(hook_path(repo.path()).exists(), "hook must land in the repo's own .git/hooks");
}

#[test]
fn install_hooks_outside_a_git_repo_fails_helpfully() {
    let tmp = tempfile::tempdir().unwrap();
    xtask()
        .arg("install-hooks")
        .current_dir(tmp.path())
        // make sure we don't accidentally escape into an enclosing repo
        .env("GIT_CEILING_DIRECTORIES", tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("git repository"));
}
