//! Repo automation, the cargo-xtask pattern: a plain workspace binary
//! invoked as `cargo xtask <cmd>` via the alias in `.cargo/config.toml`.
//! Deliberately std-only — see Cargo.toml.
//!
//! `ci` mirrors .github/workflows/ci.yml's test job (keep the two in
//! sync); the CI gate is the binding enforcement, this is the local
//! convenience that keeps pushes from ever hitting it.

use std::path::PathBuf;
use std::process::{Command, exit};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci [--dry-run]   run the full CI pipeline locally (fmt --check, clippy,
                   build, test, UI tests); --dry-run prints the commands
  fmt              apply formatting (cargo fmt --all)
  build-ui         build the React UI (npm ci + vite build) in ui/
  test-ui          run the React UI unit tests (vitest) in ui/
  viewer-dev       run braid-viewer in Tauri dev mode (`cargo tauri dev`)
                   requires `cargo install tauri-cli --version '^2'`
  viewer-build     build the braid-viewer Tauri app bundle (`cargo tauri build`);
                   args after `--` pass through, e.g.
                   `cargo xtask viewer-build -- --target <triple> --bundles dmg`
  docs             build the documentation site (mdBook -> book/)
                   requires `cargo install mdbook`
  docs-serve       preview the docs site with live reload (`mdbook serve --open`)
  install-hooks    write a .git/hooks/pre-push that runs `cargo xtask ci`
                   (opt-in; skippable with `git push --no-verify`)";

/// The pipeline `ci` runs, cheapest first; build-before-test avoids
/// relinking the braid binary under assert_cmd e2e tests (same reason as
/// ci.yml — see strand br-cache-flake).
///
/// No `--workspace` flag: Cargo uses `default-members`, which excludes
/// `braid-viewer` (Tauri can't build on musl). Build it explicitly via
/// `cargo xtask viewer-build` or `-p braid-viewer`.
const CI_STEPS: &[&[&str]] = &[
    &["cargo", "fmt", "--all", "--check"],
    &["cargo", "clippy", "--all-targets", "--", "-D", "warnings"],
    &["cargo", "build", "--all-targets"],
    &["cargo", "test"],
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("ci") => ci(&args[1..]),
        Some("fmt") => run_steps(&[&["cargo", "fmt", "--all"]]),
        Some("build-ui") => build_ui(),
        Some("test-ui") => test_ui(),
        Some("viewer-dev") => viewer_tauri("dev", &args[1..]),
        Some("viewer-build") => viewer_tauri("build", &args[1..]),
        Some("docs") => mdbook("build", &[]),
        Some("docs-serve") => mdbook("serve", &["--open"]),
        Some("install-hooks") => install_hooks(),
        Some(other) => {
            eprintln!("xtask: unknown command {other:?}\n{USAGE}");
            2
        }
        None => {
            eprintln!("{USAGE}");
            2
        }
    };
    exit(code);
}

fn ci(flags: &[String]) -> i32 {
    match flags {
        [] => {
            // Cargo pipeline first; the cargo build already runs `npm ci`
            // (braid's build.rs), so node_modules is present for the UI tests.
            let code = run_steps(CI_STEPS);
            if code != 0 {
                return code;
            }
            test_ui()
        }
        [f] if f == "--dry-run" => {
            for step in CI_STEPS {
                println!("{}", step.join(" "));
            }
            println!("npm --prefix ui run test");
            0
        }
        other => {
            eprintln!("xtask ci: unexpected arguments {other:?}\n{USAGE}");
            2
        }
    }
}

/// Run each command with inherited stdio, stopping at the first failure.
fn run_steps(steps: &[&[&str]]) -> i32 {
    for step in steps {
        let pretty = step.join(" ");
        eprintln!("xtask: {pretty}");
        match Command::new(step[0]).args(&step[1..]).status() {
            Ok(st) if st.success() => {}
            Ok(st) => {
                eprintln!("xtask: FAILED ({st}): {pretty}");
                return 1;
            }
            Err(e) => {
                eprintln!("xtask: cannot run {pretty}: {e}");
                return 1;
            }
        }
    }
    eprintln!("xtask: all steps passed");
    0
}

// ---------------------------------------------------------------------------
// build-ui / test-ui
// ---------------------------------------------------------------------------

/// Run `npm <args>` in `ui/`, returning a process-style exit code.
fn ui_npm(args: &[&str]) -> i32 {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let ui_dir = match PathBuf::from(&manifest).join("../../ui").canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("xtask: cannot find ui/ directory: {e}");
            return 1;
        }
    };
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let pretty = format!("npm {}", args.join(" "));
    eprintln!("xtask: {pretty} (in {})", ui_dir.display());
    match Command::new(npm).args(args).current_dir(&ui_dir).status() {
        Ok(st) if st.success() => 0,
        Ok(st) => {
            eprintln!("xtask: FAILED ({st}): {pretty}");
            1
        }
        Err(e) => {
            eprintln!("xtask: cannot run {pretty}: {e}");
            eprintln!("xtask: is Node.js / npm installed?");
            1
        }
    }
}

/// Build the React UI in `ui/` using npm.
///
/// The built output (`ui/dist/`) is committed to the repository so that
/// `cargo build` works without Node.js — only run this when UI source changes.
fn build_ui() -> i32 {
    for args in [&["ci"][..], &["run", "build"][..]] {
        let code = ui_npm(args);
        if code != 0 {
            return code;
        }
    }
    eprintln!("xtask: UI built — commit ui/dist/ if the output changed");
    0
}

/// Run the React UI unit tests (vitest, headless).
fn test_ui() -> i32 {
    ui_npm(&["run", "test"])
}

// ---------------------------------------------------------------------------
// viewer-dev / viewer-build
// ---------------------------------------------------------------------------

/// Pure preflight for `viewer-dev`/`viewer-build`: given whether the
/// prerequisites are present, return `Ok` when ready or a human-readable
/// error spelling out the exact install command for each missing piece.
///
/// Kept side-effect-free so it can be unit-tested without a real toolchain;
/// `viewer_tauri` does the actual probing and feeds the booleans in.
fn viewer_preflight(has_tauri_cli: bool, has_node_modules: bool) -> Result<(), String> {
    let mut problems: Vec<&str> = Vec::new();
    if !has_tauri_cli {
        problems.push(
            "  - cargo-tauri CLI not found. Install it once:\n      \
             cargo install tauri-cli --version '^2' --locked",
        );
    }
    if !has_node_modules {
        problems.push(
            "  - ui/node_modules missing (the Vite frontend deps). Install them:\n      \
             cd ui && npm install",
        );
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "xtask: cannot run the viewer — prerequisites missing:\n{}",
            problems.join("\n")
        ))
    }
}

/// Build the argv for `cargo tauri <subcommand>`, appending caller
/// passthrough args. A single leading `--` separator (as inserted by
/// `cargo xtask viewer-build -- …`) is dropped so it never reaches
/// `cargo tauri`.
fn tauri_argv(subcommand: &str, extra: &[String]) -> Vec<String> {
    let mut argv = vec!["tauri".to_string(), subcommand.to_string()];
    let extra = match extra.split_first() {
        Some((first, rest)) if first == "--" => rest,
        _ => extra,
    };
    argv.extend(extra.iter().cloned());
    argv
}

/// Run `cargo tauri <subcommand>` inside `crates/braid-viewer/`.
fn viewer_tauri(subcommand: &str, extra: &[String]) -> i32 {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let root = PathBuf::from(&manifest).join("../..");
    let viewer_dir = match root.join("crates/braid-viewer").canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("xtask: cannot find crates/braid-viewer: {e}");
            return 1;
        }
    };

    // Preflight: fail fast with an actionable message rather than letting
    // `cargo` emit a bare `no such command: tauri`, and before the Tauri CLI
    // trips over a missing `ui/node_modules` mid-build.
    let has_tauri_cli = Command::new("cargo")
        .args(["tauri", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let has_node_modules = root.join("ui/node_modules").is_dir();
    if let Err(msg) = viewer_preflight(has_tauri_cli, has_node_modules) {
        eprintln!("{msg}");
        return 1;
    }

    let argv = tauri_argv(subcommand, extra);
    eprintln!("xtask: cargo {} in {}", argv.join(" "), viewer_dir.display());
    match Command::new("cargo").args(&argv).current_dir(&viewer_dir).status() {
        Ok(st) if st.success() => 0,
        Ok(st) => {
            eprintln!("xtask: FAILED ({st}): cargo {}", argv.join(" "));
            1
        }
        Err(e) => {
            eprintln!("xtask: cannot run cargo {}: {e}", argv.join(" "));
            eprintln!(
                "xtask: is tauri-cli installed? \
                 (cargo install tauri-cli --version '^2')"
            );
            1
        }
    }
}

// ---------------------------------------------------------------------------
// docs / docs-serve
// ---------------------------------------------------------------------------

/// Run `mdbook <subcommand> [extra...]` from the repo root, where `book.toml`
/// lives (the book is built from `docs/`). Used by `docs` (build) and
/// `docs-serve` (live preview).
fn mdbook(subcommand: &str, extra: &[&str]) -> i32 {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let root = match PathBuf::from(&manifest).join("../..").canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("xtask: cannot find repo root: {e}");
            return 1;
        }
    };
    let pretty = format!("mdbook {subcommand} {}", extra.join(" "));
    eprintln!("xtask: {} (in {})", pretty.trim_end(), root.display());
    match Command::new("mdbook").arg(subcommand).args(extra).current_dir(&root).status() {
        Ok(st) if st.success() => 0,
        Ok(st) => {
            eprintln!("xtask: FAILED ({st}): {}", pretty.trim_end());
            1
        }
        Err(e) => {
            eprintln!("xtask: cannot run mdbook: {e}");
            eprintln!("xtask: is mdBook installed? (cargo install mdbook)");
            1
        }
    }
}

// ---------------------------------------------------------------------------
// install-hooks
// ---------------------------------------------------------------------------
/// anything else is refused.
const HOOK_MARKER: &str = "installed by `cargo xtask install-hooks`";

fn hook_content() -> String {
    format!(
        "#!/bin/sh\n\
         # {HOOK_MARKER} — safe to delete; reinstall any time.\n\
         echo \"pre-push: running cargo xtask ci (skip with git push --no-verify)\"\n\
         exec cargo xtask ci\n"
    )
}

fn install_hooks() -> i32 {
    // --git-path resolves correctly under worktrees and core.hooksPath.
    let out = match Command::new("git").args(["rev-parse", "--git-path", "hooks"]).output() {
        Ok(out) if out.status.success() => out,
        Ok(_) => {
            eprintln!("xtask: not inside a git repository (git rev-parse failed)");
            return 1;
        }
        Err(e) => {
            eprintln!("xtask: cannot run git: {e}");
            return 1;
        }
    };
    let hooks_dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
    let hook = hooks_dir.join("pre-push");

    match std::fs::read_to_string(&hook) {
        Ok(existing) if !existing.contains(HOOK_MARKER) => {
            eprintln!(
                "xtask: {} already exists and was not installed by xtask; \
                 not touching it. Remove or merge it yourself, then re-run.",
                hook.display()
            );
            return 1;
        }
        _ => {} // absent, unreadable-as-text, or ours: (re)write below
    }

    if let Err(e) = std::fs::create_dir_all(&hooks_dir) {
        eprintln!("xtask: cannot create {}: {e}", hooks_dir.display());
        return 1;
    }
    if let Err(e) = std::fs::write(&hook, hook_content()) {
        eprintln!("xtask: cannot write {}: {e}", hook.display());
        return 1;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)) {
            eprintln!("xtask: cannot chmod {}: {e}", hook.display());
            return 1;
        }
    }
    println!("installed {}", hook.display());
    0
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::tauri_argv;
    use super::viewer_preflight;

    fn as_strs(v: &[String]) -> Vec<&str> {
        v.iter().map(String::as_str).collect()
    }

    #[test]
    fn tauri_argv_bare_subcommand() {
        assert_eq!(as_strs(&tauri_argv("build", &[])), ["tauri", "build"]);
    }

    #[test]
    fn tauri_argv_appends_passthrough() {
        let extra = vec!["--target".to_string(), "aarch64-apple-darwin".to_string()];
        assert_eq!(
            as_strs(&tauri_argv("build", &extra)),
            ["tauri", "build", "--target", "aarch64-apple-darwin"]
        );
    }

    #[test]
    fn tauri_argv_strips_leading_separator() {
        // `cargo xtask viewer-build -- --bundles dmg` delivers a leading
        // `--` in the passthrough; it must not reach `cargo tauri`.
        let extra = vec!["--".to_string(), "--bundles".to_string(), "dmg".to_string()];
        assert_eq!(as_strs(&tauri_argv("build", &extra)), ["tauri", "build", "--bundles", "dmg"]);
    }

    #[test]
    fn preflight_ok_when_prerequisites_present() {
        assert!(viewer_preflight(true, true).is_ok());
    }

    #[test]
    fn preflight_flags_missing_tauri_cli_with_install_command() {
        let err = viewer_preflight(false, true).unwrap_err();
        assert!(err.contains("cargo install tauri-cli"), "must name the fix:\n{err}");
        assert!(!err.contains("npm install"), "must not mention node when node is fine:\n{err}");
    }

    #[test]
    fn preflight_flags_missing_node_modules_with_install_command() {
        let err = viewer_preflight(true, false).unwrap_err();
        assert!(err.contains("npm install"), "must name the fix:\n{err}");
        assert!(!err.contains("tauri-cli"), "must not mention tauri when cli is present:\n{err}");
    }

    #[test]
    fn preflight_reports_both_when_both_missing() {
        let err = viewer_preflight(false, false).unwrap_err();
        assert!(
            err.contains("cargo install tauri-cli") && err.contains("npm install"),
            "must list both fixes:\n{err}"
        );
    }

    #[test]
    fn tauri_conf_has_no_hardcoded_version() {
        // braid-viewer's tauri.conf.json must omit `version` so Tauri
        // inherits the workspace crate version; a hardcoded value drifts
        // from the release tag. See
        // claude-notes/plans/2026/06/18/viewer-release-design.md.
        let conf = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../braid-viewer/tauri.conf.json");
        let content = std::fs::read_to_string(&conf)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", conf.display()));
        assert!(
            !content.contains("\"version\""),
            "tauri.conf.json must not hardcode a version (inherit the \
             workspace crate version instead); found in {}",
            conf.display()
        );
    }
}
