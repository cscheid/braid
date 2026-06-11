//! Build script for the braid binary crate.
//!
//! Compiles the React UI (`ui/`) into `ui/dist/` before `rust-embed`
//! ingests it. Cargo only re-runs this script when UI source files change,
//! so the cost is paid once per change — not on every `cargo build`.
//!
//! Environment variables
//! ---------------------
//! SKIP_UI_BUILD=1   Skip the npm build entirely. If `ui/dist/` does not
//!                   exist a stub page is written so rust-embed can still
//!                   compile.  Use this in CI jobs that test only Rust code
//!                   and don't have Node.js available.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ui_dir = manifest.join("../../ui");
    let dist_dir = ui_dir.join("dist");

    // Re-run whenever UI source changes (Cargo skips this script otherwise).
    for rel in ["src", "package.json", "vite.config.ts", "index.html"] {
        println!("cargo:rerun-if-changed={}", ui_dir.join(rel).display());
    }
    // Also re-run if SKIP_UI_BUILD is toggled.
    println!("cargo:rerun-if-env-changed=SKIP_UI_BUILD");

    if std::env::var("SKIP_UI_BUILD").is_ok() {
        ensure_stub(&dist_dir);
        return;
    }

    let npm = npm_cmd();

    run_npm(&npm, &["ci"], &ui_dir);
    run_npm(&npm, &["run", "build"], &ui_dir);
}

fn run_npm(npm: &str, args: &[&str], cwd: &Path) {
    let pretty = format!("npm {}", args.join(" "));
    eprintln!("cargo:warning=braid build.rs: {pretty}");

    let status = Command::new(npm)
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "\n\nbraid build.rs: failed to run `{pretty}`: {e}\n\
                 Is Node.js / npm installed? If you want to build braid \
                 without Node.js, set SKIP_UI_BUILD=1 (the `braid ui` \
                 command will show a stub page).\n"
            )
        });

    assert!(
        status.success(),
        "\n\nbraid build.rs: `{pretty}` failed (exit {status}).\n\
         Fix the UI build error above, or set SKIP_UI_BUILD=1 to skip it.\n"
    );
}

/// Write a minimal stub so rust-embed has something to compile against
/// when the UI has not been built.
fn ensure_stub(dist: &Path) {
    if dist.join("index.html").exists() {
        return;
    }
    std::fs::create_dir_all(dist).expect("could not create ui/dist/");
    std::fs::write(
        dist.join("index.html"),
        "<!doctype html><html><body>\
         <h2>braid ui</h2>\
         <p>The React UI was not compiled into this binary \
         (<code>SKIP_UI_BUILD=1</code> was set at build time).</p>\
         <p>Rebuild without that variable, or run \
         <code>cargo xtask build-ui &amp;&amp; cargo build</code>.</p>\
         </body></html>",
    )
    .expect("could not write ui/dist/index.html stub");
}

fn npm_cmd() -> &'static str {
    if cfg!(windows) { "npm.cmd" } else { "npm" }
}
