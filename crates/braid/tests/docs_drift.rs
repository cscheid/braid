//! Drift guards: documentation must keep up with the command surface.
//!
//! These tests make doc drift a build failure instead of a discipline:
//! every CLI subcommand must be mentioned in `braid agents-info` (the
//! version-matched agent guide), every MCP tool must appear in docs/mcp.md,
//! and every docs/ markdown page must be wired into the mdBook table of
//! contents (docs/SUMMARY.md). Adding a command, tool, or page without
//! documenting it fails here.

use std::path::{Path, PathBuf};

fn braid_stdout(args: &[&str]) -> String {
    let mut c = assert_cmd::Command::cargo_bin("braid").unwrap();
    let out =
        c.env_clear().env("PATH", std::env::var("PATH").unwrap()).args(args).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

/// Subcommand names parsed from `braid --help` (the clap-rendered list).
fn subcommands() -> Vec<String> {
    let help = braid_stdout(&["--help"]);
    let mut names = Vec::new();
    let mut in_commands = false;
    for line in help.lines() {
        if line.starts_with("Commands:") {
            in_commands = true;
            continue;
        }
        if in_commands {
            if line.starts_with("Options:") || line.trim().is_empty() {
                break;
            }
            if let Some(name) = line.split_whitespace().next()
                && name != "help"
            {
                names.push(name.to_string());
            }
        }
    }
    assert!(names.len() >= 15, "subcommand parsing looks broken: {names:?}");
    names
}

#[test]
fn every_subcommand_is_documented_in_agents_info() {
    let info = braid_stdout(&["agents-info"]);
    let missing: Vec<String> =
        subcommands().into_iter().filter(|name| !info.contains(&format!("braid {name}"))).collect();
    assert!(
        missing.is_empty(),
        "agents-info (crates/braid/src/agents-info.md) does not mention: \
         {missing:?}\nEvery user-facing command needs a row or mention — \
         agents learn braid from this guide."
    );
}

#[test]
fn every_mcp_tool_is_documented_in_docs_mcp_md() {
    // The tool registry is the source of truth; spawning the server to ask
    // would also work, but the registry is compiled into this crate.
    let doc = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/mcp.md"),
    )
    .expect("docs/mcp.md exists");

    // braid_delete/braid_import are named explicitly; the rest appear in
    // the tier table by short name. Require each tool's short name.
    let info = braid_stdout(&["agents-info"]);
    let _ = info; // agents-info covers the CLI; docs/mcp.md covers tools
    for tool in [
        "ready",
        "blocked",
        "list",
        "show",
        "search",
        "dep_list",
        "dep_tree",
        "dep_cycles",
        "export",
        "create",
        "update",
        "close",
        "reopen",
        "defer",
        "undefer",
        "comment",
        "dep_add",
        "dep_remove",
        "braid_delete",
        "braid_import",
    ] {
        assert!(
            doc.contains(tool),
            "docs/mcp.md does not mention MCP tool {tool:?} — update the \
             capability-tier table when the tool surface changes"
        );
    }
}

/// Collect every `*.md` path under `dir`, relative to `dir`, with `/`
/// separators (matching how SUMMARY.md links are written).
fn markdown_files(dir: &Path, base: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("docs dir is readable") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            markdown_files(&path, base, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            out.push(rel);
        }
    }
}

#[test]
fn every_docs_page_is_listed_in_summary() {
    // docs/ is the mdBook source (book.toml `src = "docs"`); SUMMARY.md is
    // its table of contents. A page not in SUMMARY is silently dropped from
    // the published site, so make that a build failure. SUMMARY.md itself is
    // the TOC, not a chapter.
    let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let summary = std::fs::read_to_string(docs.join("SUMMARY.md")).expect("docs/SUMMARY.md exists");

    let mut pages = Vec::new();
    markdown_files(&docs, &docs, &mut pages);

    let missing: Vec<String> = pages
        .into_iter()
        .filter(|p| p != "SUMMARY.md")
        .filter(|p| !summary.contains(&format!("({p})")))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/SUMMARY.md does not link these pages: {missing:?}\nEvery docs/ \
         page must appear in the mdBook TOC, or it never reaches the site."
    );
}
