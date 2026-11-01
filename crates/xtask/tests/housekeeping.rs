//! e2e tests for `cargo xtask housekeeping`.
//!
//! Each test gets a scratch git repository and a `PATH` holding only git and
//! the fake tools it asks for, so a tool the test doesn't install is really
//! missing, and nothing touches the network. Fake tools are shell scripts,
//! hence the unix gate (CI's test job runs on Linux and macOS).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

struct Fixture {
    repo: TempDir,
    bin: TempDir,
    cargo_home: TempDir,
}

impl Fixture {
    /// A repo whose base commit (`main`) holds `base_lock`, with `current_lock`
    /// in the working tree. All lockfile tests compare those two.
    fn new(base_lock: &str, current_lock: &str) -> Self {
        let fixture = Fixture {
            repo: tempfile::tempdir().unwrap(),
            bin: tempfile::tempdir().unwrap(),
            cargo_home: tempfile::tempdir().unwrap(),
        };
        let git = which("git");
        std::os::unix::fs::symlink(git, fixture.bin.path().join("git")).unwrap();

        fixture.git(&["init", "-q", "-b", "main"]);
        fixture.write("Cargo.lock", base_lock);
        fixture.write("README.md", "# scratch\n");
        fixture.git(&["add", "."]);
        fixture.git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "base"]);
        fixture.write("Cargo.lock", current_lock);
        fixture
    }

    fn unchanged() -> Self {
        let lock = lockfile(&[("serde", "1.0.0", Some(CRATES_IO), "aaa")]);
        Self::new(&lock, &lock)
    }

    fn git(&self, args: &[&str]) {
        let st =
            Command::new(which("git")).args(args).current_dir(self.repo.path()).status().unwrap();
        assert!(st.success(), "git {args:?} failed");
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.repo.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    /// Install a fake tool: a /bin/sh script body with `$@` as its arguments.
    fn tool(&self, name: &str, body: &str) {
        let path = self.bin.path().join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Lay out `name-version` as cargo's registry cache would.
    fn registry_crate(&self, name: &str, version: &str, files: &[(&str, &str)]) {
        let dir = self.cargo_home.path().join(format!("registry/src/index-abc/{name}-{version}"));
        std::fs::create_dir_all(&dir).unwrap();
        for (file, content) in files {
            std::fs::write(dir.join(file), content).unwrap();
        }
    }

    fn command(&self, args: &[&str]) -> assert_cmd::Command {
        let mut cmd = assert_cmd::Command::cargo_bin("xtask").unwrap();
        cmd.env_clear()
            .env("PATH", self.bin.path())
            .env("CARGO_HOME", self.cargo_home.path())
            .env("HOME", self.cargo_home.path())
            .current_dir(self.repo.path())
            .arg("housekeeping")
            .args(["--base", "main"])
            .args(args);
        cmd
    }

    /// Run expecting exit 0 and return the report.
    fn report(&self, args: &[&str]) -> String {
        let out = self.command(args).assert().success();
        String::from_utf8(out.get_output().stdout.clone()).unwrap()
    }
}

fn which(tool: &str) -> PathBuf {
    let path = std::env::var_os("PATH").unwrap();
    std::env::split_paths(&path)
        .map(|dir| dir.join(tool))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| panic!("{tool} not on PATH"))
}

fn lockfile(packages: &[(&str, &str, Option<&str>, &str)]) -> String {
    let mut out = String::from("version = 4\n");
    for (name, version, source, checksum) in packages {
        out.push_str(&format!("\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n"));
        if let Some(source) = source {
            out.push_str(&format!("source = \"{source}\"\nchecksum = \"{checksum}\"\n"));
        }
    }
    out
}

/// The summary-table status of `check`.
fn status_of(report: &str, check: &str) -> String {
    let prefix = format!("| {check} | ");
    let line = report
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no summary row for {check} in:\n{report}"));
    line[prefix.len()..].split(" |").next().unwrap().to_string()
}

fn assert_contains(haystack: &str, needle: &str) {
    assert!(haystack.contains(needle), "expected {needle:?} in:\n{haystack}");
}

// ---------------------------------------------------------------------------
// arguments and exit codes
// ---------------------------------------------------------------------------

#[test]
fn unknown_flag_is_a_usage_error() {
    Fixture::unchanged().command(&["--frobnicate"]).assert().code(2);
}

#[test]
fn unknown_check_name_is_a_usage_error() {
    let fixture = Fixture::unchanged();
    let out = fixture.command(&["--only", "bogus"]).assert().code(2);
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert_contains(&stderr, "bogus");
}

#[test]
fn only_restricts_the_checks_that_run() {
    let report = Fixture::unchanged().report(&["--offline", "--only", "lockfile,suppressions"]);
    assert_eq!(status_of(&report, "lockfile"), "clean");
    assert_eq!(status_of(&report, "suppressions"), "clean");
    assert!(!report.contains("| links |"), "links must not run:\n{report}");
}

#[test]
fn a_failing_check_exits_1_but_the_others_still_report() {
    let fixture = Fixture::unchanged();
    let out = fixture
        .command(&["--offline", "--base", "no-such-ref", "--only", "lockfile,suppressions"])
        .assert()
        .code(1);
    let report = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(status_of(&report, "lockfile"), "error");
    assert_eq!(status_of(&report, "suppressions"), "clean");
}

// ---------------------------------------------------------------------------
// lockfile
// ---------------------------------------------------------------------------

#[test]
fn lockfile_reports_changes_additions_and_removals() {
    let base = lockfile(&[
        ("bumped", "1.0.0", Some(CRATES_IO), "a"),
        ("gone", "1.0.0", Some(CRATES_IO), "b"),
        ("same", "1.0.0", Some(CRATES_IO), "c"),
    ]);
    let current = lockfile(&[
        ("bumped", "1.1.0", Some(CRATES_IO), "d"),
        ("fresh", "2.0.0", Some(CRATES_IO), "e"),
        ("same", "1.0.0", Some(CRATES_IO), "c"),
    ]);
    let report = Fixture::new(&base, &current).report(&["--offline", "--only", "lockfile"]);
    assert_eq!(status_of(&report, "lockfile"), "findings");
    assert_contains(&report, "`bumped` 1.0.0 → 1.1.0");
    assert_contains(&report, "`fresh` 2.0.0 (new)");
    assert_contains(&report, "`gone` 1.0.0 (removed)");
    assert!(!report.contains("`same`"), "unchanged crates must not be listed:\n{report}");
}

#[test]
fn lockfile_ignores_workspace_members() {
    let base = lockfile(&[("braid", "0.8.0", None, "")]);
    let current = lockfile(&[("braid", "0.8.1", None, "")]);
    let report = Fixture::new(&base, &current).report(&["--offline", "--only", "lockfile"]);
    assert_eq!(status_of(&report, "lockfile"), "clean");
}

#[test]
fn lockfile_flags_non_crates_io_sources() {
    let base = lockfile(&[("dep", "1.0.0", Some(CRATES_IO), "a")]);
    let current = lockfile(&[("dep", "1.0.1", Some("git+https://example.com/dep#abc"), "")]);
    let report = Fixture::new(&base, &current).report(&["--offline", "--only", "lockfile"]);
    assert_contains(&report, "non-crates.io source");
}

#[test]
fn lockfile_flags_a_changed_checksum_at_the_same_version() {
    let base = lockfile(&[("dep", "1.0.0", Some(CRATES_IO), "a")]);
    let current = lockfile(&[("dep", "1.0.0", Some(CRATES_IO), "b")]);
    let report = Fixture::new(&base, &current).report(&["--offline", "--only", "lockfile"]);
    assert_eq!(status_of(&report, "lockfile"), "findings");
    assert_contains(&report, "checksum changed");
}

#[test]
fn lockfile_flags_build_scripts_and_proc_macros_from_the_registry_cache() {
    let base = lockfile(&[]);
    let current = lockfile(&[
        ("plain", "1.0.0", Some(CRATES_IO), "a"),
        ("scripted", "1.0.0", Some(CRATES_IO), "b"),
        ("declared", "1.0.0", Some(CRATES_IO), "c"),
        ("macro", "1.0.0", Some(CRATES_IO), "d"),
        ("uncached", "1.0.0", Some(CRATES_IO), "e"),
    ]);
    let fixture = Fixture::new(&base, &current);
    // Published manifests spell "no build script" as `build = false`.
    fixture.registry_crate("plain", "1.0.0", &[("Cargo.toml", "[package]\nbuild = false\n")]);
    fixture.registry_crate(
        "scripted",
        "1.0.0",
        &[("Cargo.toml", "[package]\n"), ("build.rs", "fn main() {}\n")],
    );
    fixture.registry_crate("declared", "1.0.0", &[("Cargo.toml", "[package]\nbuild = \"b.rs\"\n")]);
    fixture.registry_crate("macro", "1.0.0", &[("Cargo.toml", "[lib]\nproc-macro = true\n")]);

    let report = fixture.report(&["--offline", "--only", "lockfile"]);
    let row = |name: &str| {
        let marker = format!("`{name}` ");
        report.lines().find(|l| l.contains(&marker)).unwrap().to_string()
    };
    assert!(!row("plain").contains("build script"), "{}", row("plain"));
    assert_contains(&row("scripted"), "build script");
    assert_contains(&row("declared"), "build script");
    assert_contains(&row("macro"), "proc macro");
    assert_contains(&row("uncached"), "not inspected");
}

#[test]
fn lockfile_flags_recently_published_versions_when_online() {
    let base = lockfile(&[]);
    let current = lockfile(&[
        ("young", "1.0.0", Some(CRATES_IO), "a"),
        ("mature", "1.0.0", Some(CRATES_IO), "b"),
    ]);
    let fixture = Fixture::new(&base, &current);
    fixture.tool("cargo", "exit 0");
    fixture.tool(
        "curl",
        r#"case "$*" in
  *young*) echo '{"version":{"created_at":"2999-01-01T00:00:00Z"}}' ;;
  *)       echo '{"version":{"created_at":"2020-01-01T00:00:00Z"}}' ;;
esac"#,
    );
    let report = fixture.report(&["--only", "lockfile"]);
    let row = |name: &str| {
        let marker = format!("`{name}` ");
        report.lines().find(|l| l.contains(&marker)).unwrap().to_string()
    };
    assert_contains(&row("young"), "published under 7 days ago");
    assert!(!row("mature").contains("published under"), "{}", row("mature"));
}

// ---------------------------------------------------------------------------
// tool-backed checks
// ---------------------------------------------------------------------------

#[test]
fn missing_tools_are_skipped_visibly_with_an_install_hint() {
    let fixture = Fixture::unchanged();
    let report = fixture.report(&["--offline", "--only", "advisories,unused,links"]);
    for check in ["advisories", "unused", "links"] {
        assert_eq!(status_of(&report, check), "skipped");
    }
    assert_contains(&report, "cargo install cargo-audit");
    assert_contains(&report, "cargo install cargo-machete");
    assert_contains(&report, "lychee");
}

#[test]
fn offline_skips_network_only_checks_and_never_calls_curl() {
    let base = lockfile(&[]);
    let current = lockfile(&[("dep", "1.0.0", Some(CRATES_IO), "a")]);
    let fixture = Fixture::new(&base, &current);
    let marker = fixture.repo.path().join("curl-was-called");
    fixture.tool("curl", &format!("echo called > {}", marker.display()));
    fixture.tool("cargo", "exit 0");
    fixture.write("ui/package.json", "{}");
    fixture.tool("npm", "echo should-not-run; exit 1");

    let report =
        fixture.report(&["--offline", "--only", "lockfile,updates,npm-outdated,npm-audit"]);
    for check in ["updates", "npm-outdated", "npm-audit"] {
        assert_eq!(status_of(&report, check), "skipped");
    }
    assert!(!marker.exists(), "--offline must not call curl");
    assert!(!report.contains("should-not-run"), "{report}");
}

#[test]
fn updates_lists_only_actionable_lines() {
    let fixture = Fixture::unchanged();
    fixture.tool(
        "cargo",
        r#"case "$1" in update)
  echo '    Updating crates.io index' >&2
  echo '     Locking 2 packages to latest compatible versions' >&2
  echo '    Updating anyhow v1.0.1 -> v1.0.2' >&2
  echo '   Unchanged sha2 v0.10.9 (available: v0.11.1)' >&2
  echo 'warning: not updating lockfile due to dry run' >&2
esac"#,
    );
    let report = fixture.report(&["--only", "updates"]);
    assert_eq!(status_of(&report, "updates"), "findings");
    assert_contains(&report, "1 change(s) available, 1 held back");
    assert_contains(&report, "anyhow v1.0.1 -> v1.0.2");
    assert_contains(&report, "sha2 v0.10.9 (available: v0.11.1)");
    assert!(!report.contains("crates.io index"), "{report}");
    assert!(!report.contains("due to dry run"), "{report}");
}

#[test]
fn updates_is_clean_when_nothing_is_available() {
    let fixture = Fixture::unchanged();
    fixture.tool("cargo", "echo '    Updating crates.io index' >&2");
    assert_eq!(status_of(&fixture.report(&["--only", "updates"]), "updates"), "clean");
}

#[test]
fn advisories_distinguish_findings_from_tool_failure() {
    let fixture = Fixture::unchanged();
    fixture.tool(
        "cargo",
        r#"case "$*" in *--version*) exit 0 ;; esac
echo 'Crate:     badcrate'; exit 1"#,
    );
    let report = fixture.report(&["--only", "advisories"]);
    assert_eq!(status_of(&report, "advisories"), "findings");
    assert_contains(&report, "badcrate");
    assert_contains(&report, "1 advisory entries");

    fixture.tool(
        "cargo",
        r#"case "$*" in *--version*) exit 0 ;; esac
echo 'error: could not fetch advisory database' >&2; exit 1"#,
    );
    let out = fixture.command(&["--only", "advisories"]).assert().code(1);
    let report = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(status_of(&report, "advisories"), "error");
    assert_contains(&report, "could not fetch");
}

#[test]
fn unused_maps_machete_exit_codes() {
    for (code, status) in [(0, "clean"), (1, "findings"), (2, "error")] {
        let fixture = Fixture::unchanged();
        fixture.tool("cargo", &format!("case \"$*\" in *--version*) exit 0 ;; esac\nexit {code}"));
        let out = fixture.command(&["--only", "unused"]).assert();
        let out = out.get_output();
        let report = String::from_utf8(out.stdout.clone()).unwrap();
        assert_eq!(status_of(&report, "unused"), status, "machete exit {code}");
    }
}

#[test]
fn npm_nonzero_is_a_finding_only_when_it_produced_output() {
    let fixture = Fixture::unchanged();
    fixture.write("ui/package.json", "{}");
    fixture.tool("npm", "echo 'react 18.0.0 19.0.0'; exit 1");
    let report = fixture.report(&["--only", "npm-outdated"]);
    assert_eq!(status_of(&report, "npm-outdated"), "findings");
    assert_contains(&report, "react 18.0.0 19.0.0");

    fixture.tool("npm", "echo 'npm error ENOTFOUND' >&2; exit 1");
    let out = fixture.command(&["--only", "npm-audit"]).assert().code(1);
    let report = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(status_of(&report, "npm-audit"), "error");
}

#[test]
fn npm_checks_skip_when_there_is_no_ui_directory() {
    let fixture = Fixture::unchanged();
    fixture.tool("npm", "exit 1");
    let report = fixture.report(&["--only", "npm-outdated"]);
    assert_eq!(status_of(&report, "npm-outdated"), "skipped");
}

#[test]
fn links_map_lychee_exit_codes_and_pass_offline() {
    let fixture = Fixture::unchanged();
    let args_file = fixture.repo.path().join("lychee-args");
    fixture.tool(
        "lychee",
        &format!(
            "case \"$*\" in *--version*) exit 0 ;; esac\necho \"$*\" > {}\necho 'broken.md'\nexit 2",
            args_file.display()
        ),
    );
    let report = fixture.report(&["--offline", "--only", "links"]);
    assert_eq!(status_of(&report, "links"), "findings");
    assert_contains(&report, "broken.md");
    assert_contains(&std::fs::read_to_string(&args_file).unwrap(), "--offline");

    let report = fixture.report(&["--only", "links"]);
    assert!(!std::fs::read_to_string(&args_file).unwrap().contains("--offline"), "{report}");

    fixture.tool("lychee", "case \"$*\" in *--version*) exit 0 ;; esac\nexit 3");
    fixture.command(&["--only", "links"]).assert().code(1);
}

// ---------------------------------------------------------------------------
// repo inventories
// ---------------------------------------------------------------------------

#[test]
fn actions_flags_mixed_versions_of_the_same_action() {
    let fixture = Fixture::unchanged();
    fixture.write(
        ".github/workflows/a.yml",
        "steps:\n  - uses: actions/checkout@v5\n  - uses: ./local/action\n  - uses: foo/bar@v1 # note\n",
    );
    fixture.write(".github/workflows/b.yml", "steps:\n  - uses: \"actions/checkout@v6\"\n");
    let report = fixture.report(&["--only", "actions"]);
    assert_eq!(status_of(&report, "actions"), "findings");
    assert_contains(&report, "`actions/checkout`: mixed versions v5, v6");
    assert_contains(&report, "`foo/bar@v1`");
    assert!(!report.contains("local/action"), "local actions aren't versioned:\n{report}");
}

#[test]
fn actions_is_informational_when_versions_agree() {
    let fixture = Fixture::unchanged();
    fixture.write(".github/workflows/a.yml", "steps:\n  - uses: foo/bar@v1\n");
    assert_eq!(status_of(&fixture.report(&["--only", "actions"]), "actions"), "info");
}

#[test]
fn suppressions_lists_tracked_rust_files_only() {
    let fixture = Fixture::unchanged();
    fixture.write("src/lib.rs", "#[allow(dead_code)]\nfn a() {}\n// TODO: tidy\n");
    fixture.write("src/untracked.rs", "#[ignore]\n");
    fixture.write("notes.md", "TODO: not rust\n");
    fixture.git(&["add", "src/lib.rs", "notes.md"]);
    let report = fixture.report(&["--only", "suppressions"]);
    assert_eq!(status_of(&report, "suppressions"), "info");
    assert_contains(&report, "src/lib.rs:1");
    assert_contains(&report, "src/lib.rs:3");
    assert!(!report.contains("untracked.rs"), "{report}");
    assert!(!report.contains("notes.md"), "{report}");
}

#[test]
fn notes_lists_markers_in_any_tracked_text_file_except_markdown() {
    let fixture = Fixture::unchanged();
    fixture.write("crates/a/Cargo.toml", "# HOUSEKEEPING: drop this pin when x ships\na = 1\n");
    fixture.write("install.sh", "true\n# HOUSEKEEPING: remove once y is gone\n");
    fixture.write("docs/guide.md", "Mark them with `HOUSEKEEPING:` comments.\n");
    fixture.write("untracked.toml", "# HOUSEKEEPING: not tracked\n");
    fixture.git(&["add", "crates/a/Cargo.toml", "install.sh", "docs/guide.md"]);
    let report = fixture.report(&["--only", "notes"]);
    assert_eq!(status_of(&report, "notes"), "info");
    assert_contains(&report, "crates/a/Cargo.toml:1: # HOUSEKEEPING: drop this pin when x ships");
    assert_contains(&report, "install.sh:2:");
    assert!(!report.contains("guide.md"), "{report}");
    assert!(!report.contains("not tracked"), "{report}");
}

#[test]
fn notes_is_clean_without_markers() {
    let report = Fixture::unchanged().report(&["--only", "notes"]);
    assert_eq!(status_of(&report, "notes"), "clean");
}

// ---------------------------------------------------------------------------
// npm-lockfile
// ---------------------------------------------------------------------------

/// A package-lock.json with npm's layout. Each entry is
/// (path after node_modules/, version, resolved host, integrity, install script).
fn npm_lock(entries: &[(&str, &str, &str, &str, bool)]) -> String {
    let mut out = String::from(
        "{\n  \"name\": \"ui\",\n  \"lockfileVersion\": 3,\n  \"packages\": {\n    \"\": {\n      \"name\": \"ui\"\n    },\n",
    );
    for (i, (name, version, host, integrity, script)) in entries.iter().enumerate() {
        out.push_str(&format!(
            "    \"node_modules/{name}\": {{\n      \"version\": \"{version}\",\n      \"resolved\": \"https://{host}/x.tgz\",\n      \"integrity\": \"{integrity}\",\n"
        ));
        if *script {
            out.push_str("      \"hasInstallScript\": true,\n");
        }
        // Nested fields mustn't be mistaken for the package's own.
        out.push_str("      \"dependencies\": {\n        \"version\": \"9.9.9\"\n      }\n");
        out.push_str(if i + 1 == entries.len() { "    }\n" } else { "    },\n" });
    }
    out.push_str("  }\n}\n");
    out
}

const NPMJS: &str = "registry.npmjs.org";

fn npm_fixture(base: &str, current: &str) -> Fixture {
    let fixture = Fixture::unchanged();
    fixture.write("ui/package-lock.json", base);
    fixture.git(&["add", "ui/package-lock.json"]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "ui"]);
    fixture.write("ui/package-lock.json", current);
    fixture
}

#[test]
fn npm_lockfile_reports_changes_with_flags() {
    let base = npm_lock(&[
        ("bumped", "1.0.0", NPMJS, "a", false),
        ("gone", "1.0.0", NPMJS, "b", false),
        ("same", "1.0.0", NPMJS, "c", false),
        ("tampered", "1.0.0", NPMJS, "d", false),
    ]);
    let current = npm_lock(&[
        ("bumped", "1.1.0", NPMJS, "e", false),
        ("@scope/scripted", "2.0.0", NPMJS, "f", true),
        ("elsewhere", "3.0.0", "evil.example.com", "g", false),
        ("outer/node_modules/nested", "4.0.0", NPMJS, "h", false),
        ("same", "1.0.0", NPMJS, "c", false),
        ("tampered", "1.0.0", NPMJS, "i", false),
    ]);
    let report = npm_fixture(&base, &current).report(&["--offline", "--only", "npm-lockfile"]);
    let row = |name: &str| {
        let marker = format!("`{name}` ");
        report.lines().find(|l| l.contains(&marker)).unwrap_or_default().to_string()
    };
    assert_eq!(status_of(&report, "npm-lockfile"), "findings");
    assert_contains(&row("bumped"), "1.0.0 → 1.1.0");
    assert_contains(&row("gone"), "(removed)");
    assert_contains(&row("@scope/scripted"), "install script");
    assert_contains(&row("elsewhere"), "non-npmjs source");
    assert_contains(&row("outer/node_modules/nested"), "(new)");
    assert_contains(&row("tampered"), "checksum changed");
    assert!(!report.contains("`same`"), "{report}");
    assert!(!report.contains("9.9.9"), "nested field taken as the version:\n{report}");
}

#[test]
fn npm_lockfile_flags_recently_published_versions_when_online() {
    let base = npm_lock(&[]);
    let current = npm_lock(&[
        ("young", "1.0.0", NPMJS, "a", false),
        ("mature", "1.0.0", NPMJS, "b", false),
        ("@scope/pkg", "2.0.0", NPMJS, "c", false),
        ("vanished", "1.0.0", NPMJS, "d", false),
    ]);
    let fixture = npm_fixture(&base, &current);
    // A registry document holds a `time` map keyed by version.
    fixture.tool(
        "curl",
        r#"case "$*" in
  *registry.npmjs.org/young*) echo '{"time":{"1.0.0":"2999-01-01T00:00:00Z"}}' ;;
  *registry.npmjs.org/mature*) echo '{"time":{"1.0.0":"2020-01-01T00:00:00Z"}}' ;;
  *registry.npmjs.org/@scope%2fpkg*) echo '{"time":{"2.0.0":"2999-01-01T00:00:00Z"}}' ;;
  *) exit 22 ;;
esac"#,
    );
    let report = fixture.report(&["--only", "npm-lockfile"]);
    let row = |name: &str| {
        let marker = format!("`{name}` ");
        report.lines().find(|l| l.contains(&marker)).unwrap().to_string()
    };
    assert_contains(&row("young"), "published under 7 days ago");
    assert_contains(&row("@scope/pkg"), "published under 7 days ago");
    assert!(!row("mature").contains("published"), "{}", row("mature"));
    assert_contains(&row("vanished"), "publish date unknown");
}

#[test]
fn npm_lockfile_is_clean_or_skipped_when_nothing_to_compare() {
    let lock = npm_lock(&[("same", "1.0.0", NPMJS, "c", false)]);
    let report = npm_fixture(&lock, &lock).report(&["--offline", "--only", "npm-lockfile"]);
    assert_eq!(status_of(&report, "npm-lockfile"), "clean");

    let report = Fixture::unchanged().report(&["--offline", "--only", "npm-lockfile"]);
    assert_eq!(status_of(&report, "npm-lockfile"), "skipped");
}

// ---------------------------------------------------------------------------
// safety invariants
// ---------------------------------------------------------------------------

#[test]
fn child_tools_do_not_receive_secrets() {
    let fixture = Fixture::unchanged();
    fixture.tool(
        "lychee",
        r#"case "$*" in *--version*) exit 0 ;; esac
echo "doc=[$BRAID_DOC_ID] gh=[$GH_TOKEN] github=[$GITHUB_TOKEN]"
exit 2"#,
    );
    let out = fixture
        .command(&["--only", "links"])
        .env("BRAID_DOC_ID", "s3cret-doc")
        .env("GH_TOKEN", "s3cret-gh")
        .env("GITHUB_TOKEN", "s3cret-github")
        .assert()
        .success();
    let report = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_contains(&report, "doc=[] gh=[] github=[]");
}

/// Regression: `cargo run` sets CARGO_PKG_NAME for xtask, and cargo-machete
/// misparses its arguments when it sees that, so under `cargo xtask` the
/// install probe failed and the check was wrongly skipped.
#[test]
fn child_tools_do_not_inherit_cargo_runs_package_environment() {
    let fixture = Fixture::unchanged();
    fixture.tool(
        "cargo",
        r#"[ -z "$CARGO_PKG_NAME$CARGO_MANIFEST_DIR" ] || exit 2
exit 0"#,
    );
    let out = fixture
        .command(&["--only", "unused"])
        .env("CARGO_PKG_NAME", "xtask")
        .env("CARGO_MANIFEST_DIR", "/somewhere")
        .assert()
        .success();
    let report = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(status_of(&report, "unused"), "clean");
}

#[test]
fn braid_config_never_reaches_the_report() {
    let fixture = Fixture::unchanged();
    fixture.write(".braid.toml", "doc_id = \"s3cret-doc\"\n");
    let report = fixture.report(&["--offline"]);
    assert!(!report.contains("s3cret-doc"), "{report}");
}

#[test]
fn report_scrubs_local_paths() {
    let fixture = Fixture::unchanged();
    let repo = fixture.repo.path().canonicalize().unwrap();
    let home = fixture.cargo_home.path().canonicalize().unwrap();
    fixture.tool(
        "lychee",
        &format!(
            "case \"$*\" in *--version*) exit 0 ;; esac\n\
             echo 'file://{}/README.md and {}/.cargo/x'\nexit 2",
            repo.display(),
            home.display()
        ),
    );
    let out = fixture.command(&["--only", "links"]).env("HOME", &home).assert().success();
    let report = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(!report.contains(repo.to_str().unwrap()), "{report}");
    assert!(!report.contains(home.to_str().unwrap()), "{report}");
    assert_contains(&report, "file://./README.md");
    assert_contains(&report, "~/.cargo/x");
}

#[test]
fn tool_output_cannot_break_out_of_its_code_fence() {
    let fixture = Fixture::unchanged();
    fixture.tool(
        "lychee",
        "case \"$*\" in *--version*) exit 0 ;; esac\nprintf '```\\n# injected heading\\n'\nexit 2",
    );
    let report = fixture.report(&["--only", "links"]);
    assert_contains(&report, "````\n```\n# injected heading\n````");
}

#[test]
fn does_not_modify_the_working_tree() {
    let fixture = Fixture::unchanged();
    fixture.git(&["add", "."]);
    fixture.git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-qm",
        "x",
        "--allow-empty",
    ]);
    fixture.report(&["--offline"]);
    let out = Command::new(which("git"))
        .args(["status", "--porcelain"])
        .current_dir(fixture.repo.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
}

// ---------------------------------------------------------------------------
// this repository
// ---------------------------------------------------------------------------

#[test]
fn runs_against_this_repository() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = assert_cmd::Command::cargo_bin("xtask")
        .unwrap()
        .current_dir(crate_dir)
        .args(["housekeeping", "--offline", "--base", "HEAD", "--only", "lockfile,actions"])
        .assert()
        .success();
    let report = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_contains(&report, "actions/checkout");
}

/// The skill works from the report, so it has to know about every check.
#[test]
fn housekeeping_skill_covers_every_check() {
    let out = assert_cmd::Command::cargo_bin("xtask")
        .unwrap()
        .args(["housekeeping", "--frobnicate"])
        .assert()
        .code(2);
    let usage = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    let checks = usage.split("subset of: ").nth(1).expect("usage lists the checks").trim();

    let skill = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.claude/skills/braid-housekeeping/SKILL.md");
    let skill = std::fs::read_to_string(&skill)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", skill.display()));
    for check in checks.split(", ") {
        assert_contains(&skill, &format!("`{check}`"));
    }
}
