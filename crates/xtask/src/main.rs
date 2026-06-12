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
                   build, test); --dry-run prints the commands instead
  fmt              apply formatting (cargo fmt --all)
  build-ui         build the React UI (npm ci + vite build) in ui/
  install-hooks    write a .git/hooks/pre-push that runs `cargo xtask ci`
                   (opt-in; skippable with `git push --no-verify`)";

/// The pipeline `ci` runs, cheapest first; build-before-test avoids
/// relinking the braid binary under assert_cmd e2e tests (same reason as
/// ci.yml — see strand br-cache-flake).
const CI_STEPS: &[&[&str]] = &[
    &["cargo", "fmt", "--all", "--check"],
    &["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
    &["cargo", "build", "--workspace", "--all-targets"],
    &["cargo", "test", "--workspace"],
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("ci") => ci(&args[1..]),
        Some("fmt") => run_steps(&[&["cargo", "fmt", "--all"]]),
        Some("build-ui") => build_ui(),
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
        [] => run_steps(CI_STEPS),
        [f] if f == "--dry-run" => {
            for step in CI_STEPS {
                println!("{}", step.join(" "));
            }
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
// build-ui
// ---------------------------------------------------------------------------

/// Build the React UI in `ui/` using npm.
///
/// The built output (`ui/dist/`) is committed to the repository so that
/// `cargo build` works without Node.js — only run this when UI source changes.
fn build_ui() -> i32 {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let ui_dir = PathBuf::from(&manifest).join("../../ui");
    let ui_dir = match ui_dir.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("xtask: cannot find ui/ directory: {e}");
            return 1;
        }
    };

    eprintln!("xtask: building UI in {}", ui_dir.display());

    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };

    for step_args in [&["ci"][..], &["run", "build"][..]] {
        let pretty = format!("npm {}", step_args.join(" "));
        eprintln!("xtask: {pretty}");
        match Command::new(npm).args(step_args).current_dir(&ui_dir).status() {
            Ok(st) if st.success() => {}
            Ok(st) => {
                eprintln!("xtask: FAILED ({st}): {pretty}");
                return 1;
            }
            Err(e) => {
                eprintln!("xtask: cannot run {pretty}: {e}");
                eprintln!("xtask: is Node.js / npm installed?");
                return 1;
            }
        }
    }

    eprintln!("xtask: UI built — commit ui/dist/ if the output changed");
    0
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
