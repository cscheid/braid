//! E2E tests for install.sh, the curl|bash installer.
//!
//! Modeled on beads_rust's installer test harness, but offline-first:
//! every install path is exercised through `--artifact-url file://...`
//! plus `--checksum`, so no test here touches the network. (One ignored
//! test validates real version resolution once releases exist; Phase 4
//! of the installer plan runs it.)
//!
//! The installer's platform detection is tested by shimming `uname` via
//! PATH rather than by parsing the script.
//!
//! Plan: claude-notes/plans/2026/06/04/installer.md (strand br-iju0n3gd)

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn install_sh() -> PathBuf {
    repo_root().join("install.sh")
}

/// A sandbox for one installer run: its own HOME, dest dir, and a
/// deterministic PATH (system tool dirs only, optionally prefixed with a
/// shim dir so tests can fake `uname`).
struct Sandbox {
    tmp: tempfile::TempDir,
}

const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

impl Sandbox {
    fn new() -> Sandbox {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("home")).unwrap();
        Sandbox { tmp }
    }

    fn home(&self) -> PathBuf {
        self.tmp.path().join("home")
    }

    /// Install destination. Deliberately not created up front: the
    /// installer must create it.
    fn dest(&self) -> PathBuf {
        self.tmp.path().join("bin")
    }

    fn installed_binary(&self) -> PathBuf {
        self.dest().join("braid")
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_env(args, &[])
    }

    fn run_env(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new("bash");
        cmd.arg(install_sh())
            .args(args)
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", SYSTEM_PATH);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    /// Write a fake `uname` responding to -s/-m, and return a PATH that
    /// resolves it first.
    fn uname_shim(&self, os: &str, arch: &str) -> String {
        let shim = self.tmp.path().join("shim");
        fs::create_dir_all(&shim).unwrap();
        let uname = shim.join("uname");
        fs::write(
            &uname,
            format!(
                "#!/bin/sh\ncase \"${{1:-}}\" in\n  -m) echo \"{arch}\" ;;\n  *) echo \"{os}\" ;;\nesac\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&uname, fs::Permissions::from_mode(0o755)).unwrap();
        format!("{}:{}", shim.display(), SYSTEM_PATH)
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn assert_success(out: &Output) {
    assert!(
        out.status.success(),
        "expected success, got {:?}\nstdout: {}\nstderr: {}",
        out.status,
        stdout(out),
        stderr(out)
    );
}

fn assert_failure(out: &Output) {
    assert!(
        !out.status.success(),
        "expected failure, got success\nstdout: {}\nstderr: {}",
        stdout(out),
        stderr(out)
    );
}

/// Build a release-shaped artifact: a tar.gz containing a single
/// executable named `braid` (a shell script standing in for the real
/// binary). Returns the artifact's file:// URL and its SHA-256.
fn make_artifact(dir: &Path) -> (String, String) {
    let payload = dir.join("payload");
    fs::create_dir_all(&payload).unwrap();
    let bin = payload.join("braid");
    fs::write(&bin, "#!/bin/sh\necho \"braid 0.0.0-test\"\n").unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();

    let archive = dir.join("braid-0.0.0-test.tar.gz");
    let status = Command::new("tar")
        .args(["-czf"])
        .arg(&archive)
        .arg("-C")
        .arg(&payload)
        .arg("braid")
        .status()
        .unwrap();
    assert!(status.success(), "tar failed");

    let sha = Sha256::digest(fs::read(&archive).unwrap());
    (format!("file://{}", archive.display()), format!("{sha:x}"))
}

fn dest_arg(sb: &Sandbox) -> String {
    sb.dest().display().to_string()
}

// --- help & argument handling ----------------------------------------------

#[test]
fn help_lists_every_flag_and_exits_zero() {
    let sb = Sandbox::new();
    let out = sb.run(&["--help"]);
    assert_success(&out);
    let text = stdout(&out);
    for flag in [
        "--version",
        "--dest",
        "--artifact-url",
        "--checksum",
        "--insecure-skip-checksum",
        "--from-source",
        "--uninstall",
        "--print-platform",
        "--quiet",
        "--help",
        "BRAID_INSTALL_DIR",
    ] {
        assert!(text.contains(flag), "--help is missing {flag}\n{text}");
    }
}

#[test]
fn unknown_flag_is_an_error_naming_the_flag() {
    let sb = Sandbox::new();
    let out = sb.run(&["--frobnicate"]);
    assert_failure(&out);
    assert!(stderr(&out).contains("--frobnicate"), "stderr: {}", stderr(&out));
}

// --- platform detection ------------------------------------------------------

#[test]
fn detects_linux_amd64() {
    let sb = Sandbox::new();
    let path = sb.uname_shim("Linux", "x86_64");
    let out = sb.run_env(&["--print-platform"], &[("PATH", &path)]);
    assert_success(&out);
    assert_eq!(stdout(&out).trim(), "linux_amd64");
}

#[test]
fn detects_linux_arm64_from_aarch64() {
    let sb = Sandbox::new();
    let path = sb.uname_shim("Linux", "aarch64");
    let out = sb.run_env(&["--print-platform"], &[("PATH", &path)]);
    assert_success(&out);
    assert_eq!(stdout(&out).trim(), "linux_arm64");
}

#[test]
fn detects_darwin_arm64() {
    let sb = Sandbox::new();
    let path = sb.uname_shim("Darwin", "arm64");
    let out = sb.run_env(&["--print-platform"], &[("PATH", &path)]);
    assert_success(&out);
    assert_eq!(stdout(&out).trim(), "darwin_arm64");
}

#[test]
fn detects_darwin_amd64() {
    let sb = Sandbox::new();
    let path = sb.uname_shim("Darwin", "x86_64");
    let out = sb.run_env(&["--print-platform"], &[("PATH", &path)]);
    assert_success(&out);
    assert_eq!(stdout(&out).trim(), "darwin_amd64");
}

#[test]
fn unsupported_os_dies_pointing_at_cargo_install() {
    let sb = Sandbox::new();
    let path = sb.uname_shim("SunOS", "x86_64");
    let out = sb.run_env(&["--print-platform"], &[("PATH", &path)]);
    assert_failure(&out);
    assert!(stderr(&out).contains("cargo install"), "stderr: {}", stderr(&out));
}

#[test]
fn unsupported_arch_dies_pointing_at_cargo_install() {
    let sb = Sandbox::new();
    let path = sb.uname_shim("Linux", "mips64");
    let out = sb.run_env(&["--print-platform"], &[("PATH", &path)]);
    assert_failure(&out);
    assert!(stderr(&out).contains("cargo install"), "stderr: {}", stderr(&out));
}

// --- install from a local artifact -------------------------------------------

#[test]
fn installs_from_local_artifact_with_checksum() {
    let sb = Sandbox::new();
    let (url, sha) = make_artifact(sb.tmp.path());
    let out =
        sb.run(&["--artifact-url", &url, "--checksum", &sha, "--dest", &dest_arg(&sb)]);
    assert_success(&out);

    let bin = sb.installed_binary();
    assert!(bin.is_file(), "binary not installed at {bin:?}");
    let mode = fs::metadata(&bin).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111, "binary not executable: mode {mode:o}");

    let run = Command::new(&bin).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "braid 0.0.0-test");

    // Progress goes to stderr; stdout stays clean for scripting.
    assert_eq!(stdout(&out), "", "stdout should be empty");
    assert!(stderr(&out).contains("checksum verified"), "stderr: {}", stderr(&out));
}

#[test]
fn creates_dest_directory_if_missing() {
    let sb = Sandbox::new();
    let (url, sha) = make_artifact(sb.tmp.path());
    let deep = sb.tmp.path().join("a/b/c");
    let out = sb.run(&[
        "--artifact-url",
        &url,
        "--checksum",
        &sha,
        "--dest",
        &deep.display().to_string(),
    ]);
    assert_success(&out);
    assert!(deep.join("braid").is_file());
}

#[test]
fn checksum_mismatch_fails_and_installs_nothing() {
    let sb = Sandbox::new();
    let (url, _sha) = make_artifact(sb.tmp.path());
    let wrong = "0".repeat(64);
    let out =
        sb.run(&["--artifact-url", &url, "--checksum", &wrong, "--dest", &dest_arg(&sb)]);
    assert_failure(&out);
    assert!(stderr(&out).contains("mismatch"), "stderr: {}", stderr(&out));

    // Nothing installed — not the binary, not a partial file.
    if sb.dest().exists() {
        let leftovers: Vec<_> = fs::read_dir(sb.dest()).unwrap().collect();
        assert!(leftovers.is_empty(), "dest not empty: {leftovers:?}");
    }
}

#[test]
fn malformed_checksum_is_rejected() {
    let sb = Sandbox::new();
    let (url, _sha) = make_artifact(sb.tmp.path());
    let out = sb.run(&[
        "--artifact-url",
        &url,
        "--checksum",
        "not-a-sha",
        "--dest",
        &dest_arg(&sb),
    ]);
    assert_failure(&out);
    assert!(!sb.installed_binary().exists());
}

#[test]
fn missing_checksum_refuses_to_install() {
    let sb = Sandbox::new();
    let (url, _sha) = make_artifact(sb.tmp.path());
    // No --checksum and no .sha256 sidecar: fail closed.
    let out = sb.run(&["--artifact-url", &url, "--dest", &dest_arg(&sb)]);
    assert_failure(&out);
    assert!(
        stderr(&out).contains("--insecure-skip-checksum"),
        "refusal should name the escape hatch\nstderr: {}",
        stderr(&out)
    );
    assert!(!sb.installed_binary().exists());
}

#[test]
fn insecure_skip_checksum_installs_with_loud_warning() {
    let sb = Sandbox::new();
    let (url, _sha) = make_artifact(sb.tmp.path());
    let out = sb.run(&[
        "--artifact-url",
        &url,
        "--insecure-skip-checksum",
        "--dest",
        &dest_arg(&sb),
    ]);
    assert_success(&out);
    assert!(sb.installed_binary().is_file());
    assert!(
        stderr(&out).to_lowercase().contains("unverified"),
        "warning should say the install is unverified\nstderr: {}",
        stderr(&out)
    );
}

#[test]
fn checksum_sidecar_file_is_used_automatically() {
    let sb = Sandbox::new();
    let (url, sha) = make_artifact(sb.tmp.path());
    let archive_path = url.strip_prefix("file://").unwrap();
    fs::write(
        format!("{archive_path}.sha256"),
        format!("{sha}  {}\n", Path::new(archive_path).file_name().unwrap().to_string_lossy()),
    )
    .unwrap();

    let out = sb.run(&["--artifact-url", &url, "--dest", &dest_arg(&sb)]);
    assert_success(&out);
    assert!(sb.installed_binary().is_file());
    assert!(stderr(&out).contains("checksum verified"), "stderr: {}", stderr(&out));
}

#[test]
fn install_is_idempotent() {
    let sb = Sandbox::new();
    let (url, sha) = make_artifact(sb.tmp.path());
    let args = ["--artifact-url", url.as_str(), "--checksum", &sha];
    let dest = dest_arg(&sb);

    for _ in 0..2 {
        let mut all = args.to_vec();
        all.extend_from_slice(&["--dest", &dest]);
        assert_success(&sb.run(&all));
    }
    let run = Command::new(sb.installed_binary()).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "braid 0.0.0-test");
}

#[test]
fn archive_without_braid_binary_fails_cleanly() {
    let sb = Sandbox::new();
    // An archive containing some other file, but no `braid`.
    let payload = sb.tmp.path().join("other-payload");
    fs::create_dir_all(&payload).unwrap();
    fs::write(payload.join("README"), "not a binary\n").unwrap();
    let archive = sb.tmp.path().join("braid-bogus.tar.gz");
    assert!(
        Command::new("tar")
            .args(["-czf"])
            .arg(&archive)
            .arg("-C")
            .arg(&payload)
            .arg("README")
            .status()
            .unwrap()
            .success()
    );
    let sha = format!("{:x}", Sha256::digest(fs::read(&archive).unwrap()));

    let url = format!("file://{}", archive.display());
    let out = sb.run(&["--artifact-url", &url, "--checksum", &sha, "--dest", &dest_arg(&sb)]);
    assert_failure(&out);
    assert!(!sb.installed_binary().exists());
}

// --- dest resolution ---------------------------------------------------------

#[test]
fn braid_install_dir_env_overrides_default_dest() {
    let sb = Sandbox::new();
    let (url, sha) = make_artifact(sb.tmp.path());
    let env_dest = sb.tmp.path().join("env-bin");
    let out = sb.run_env(
        &["--artifact-url", &url, "--checksum", &sha],
        &[("BRAID_INSTALL_DIR", &env_dest.display().to_string())],
    );
    assert_success(&out);
    assert!(env_dest.join("braid").is_file());
}

#[test]
fn dest_flag_beats_braid_install_dir_env() {
    let sb = Sandbox::new();
    let (url, sha) = make_artifact(sb.tmp.path());
    let env_dest = sb.tmp.path().join("env-bin");
    let out = sb.run_env(
        &["--artifact-url", &url, "--checksum", &sha, "--dest", &dest_arg(&sb)],
        &[("BRAID_INSTALL_DIR", &env_dest.display().to_string())],
    );
    assert_success(&out);
    assert!(sb.installed_binary().is_file());
    assert!(!env_dest.exists());
}

// --- PATH advice -------------------------------------------------------------

#[test]
fn warns_when_dest_is_not_on_path() {
    let sb = Sandbox::new();
    let (url, sha) = make_artifact(sb.tmp.path());
    let out =
        sb.run(&["--artifact-url", &url, "--checksum", &sha, "--dest", &dest_arg(&sb)]);
    assert_success(&out);
    assert!(stderr(&out).contains("PATH"), "expected PATH advice\nstderr: {}", stderr(&out));
}

#[test]
fn no_path_warning_when_dest_is_on_path() {
    let sb = Sandbox::new();
    let (url, sha) = make_artifact(sb.tmp.path());
    let path = format!("{}:{}", sb.dest().display(), SYSTEM_PATH);
    let out = sb.run_env(
        &["--artifact-url", &url, "--checksum", &sha, "--dest", &dest_arg(&sb)],
        &[("PATH", &path)],
    );
    assert_success(&out);
    assert!(
        !stderr(&out).contains("PATH"),
        "unexpected PATH advice\nstderr: {}",
        stderr(&out)
    );
}

// --- quiet mode ----------------------------------------------------------------

#[test]
fn quiet_successful_install_prints_nothing() {
    let sb = Sandbox::new();
    let (url, sha) = make_artifact(sb.tmp.path());
    // dest on PATH so there is no legitimate warning to print.
    let path = format!("{}:{}", sb.dest().display(), SYSTEM_PATH);
    let out = sb.run_env(
        &["--quiet", "--artifact-url", &url, "--checksum", &sha, "--dest", &dest_arg(&sb)],
        &[("PATH", &path)],
    );
    assert_success(&out);
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), "");
    assert!(sb.installed_binary().is_file());
}

#[test]
fn quiet_still_reports_errors() {
    let sb = Sandbox::new();
    let (url, _sha) = make_artifact(sb.tmp.path());
    let wrong = "0".repeat(64);
    let out = sb.run(&[
        "--quiet",
        "--artifact-url",
        &url,
        "--checksum",
        &wrong,
        "--dest",
        &dest_arg(&sb),
    ]);
    assert_failure(&out);
    assert!(!stderr(&out).is_empty(), "errors must print even under --quiet");
}

// --- uninstall -----------------------------------------------------------------

#[test]
fn uninstall_removes_the_binary() {
    let sb = Sandbox::new();
    let (url, sha) = make_artifact(sb.tmp.path());
    let dest = dest_arg(&sb);
    assert_success(&sb.run(&["--artifact-url", &url, "--checksum", &sha, "--dest", &dest]));
    assert!(sb.installed_binary().is_file());

    assert_success(&sb.run(&["--uninstall", "--dest", &dest]));
    assert!(!sb.installed_binary().exists());
}

#[test]
fn uninstall_when_nothing_installed_succeeds_with_notice() {
    let sb = Sandbox::new();
    let out = sb.run(&["--uninstall", "--dest", &dest_arg(&sb)]);
    assert_success(&out);
    assert!(!stderr(&out).is_empty(), "expected a nothing-to-remove notice");
}

// --- script hygiene ------------------------------------------------------------

#[test]
fn shellcheck_clean_if_available() {
    let shellcheck = Command::new("shellcheck").arg("--version").output();
    if shellcheck.is_err() {
        eprintln!("shellcheck not installed; skipping");
        return;
    }
    let out = Command::new("shellcheck")
        .arg("--severity=style")
        .arg(install_sh())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "shellcheck findings:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

// --- network (run manually / in Phase 4 once a release exists) ------------------

#[test]
#[ignore = "needs a published GitHub release; run in installer plan Phase 4"]
fn resolves_latest_version_from_github() {
    let sb = Sandbox::new();
    let (_, sha) = make_artifact(sb.tmp.path());
    let _ = sha;
    // Plain install with no --version/--artifact-url: resolves the latest
    // release, downloads, verifies the published .sha256, installs.
    let out = sb.run(&["--dest", &dest_arg(&sb)]);
    assert_success(&out);
    let run = Command::new(sb.installed_binary()).arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&run.stdout).starts_with("braid"));
}
