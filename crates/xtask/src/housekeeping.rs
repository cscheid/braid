//! `cargo xtask housekeeping`: the directly automatable part of periodic repo
//! chores, printed as a Markdown report for a human or agent to act on.
//!
//! Invariants, which are the point of having this be code rather than prose
//! in a skill (see claude-notes/plans/2026/11/01/xtask-housekeeping.md):
//!
//! - Runs no dependency code: no `cargo build`/`test`, no npm install, and
//!   xtask itself is std-only. Safe to run on an unreviewed dependency bump.
//! - Read-only: never changes the repo. `cargo fetch` only fills the registry
//!   cache, and `cargo update` is always `--dry-run`.
//! - Stays away from secrets: never reads `.braid.toml` or runs `braid`, and
//!   child processes don't inherit braid or GitHub credentials
//!   (see `is_withheld_env`).
//! - The report is safe to publish: local paths are scrubbed and tool output
//!   is fenced.
//! - Never silently passes: a check that couldn't run says so in the summary.
//!
//! Exit codes: 0 every check ran (findings and skips included), 1 a check
//! errored, 2 usage error.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const USAGE: &str = "\
usage: cargo xtask housekeeping [--base <ref>] [--offline] [--only <check,...>]

  --base <ref>       compare lockfiles against this ref (default: origin/main)
  --offline          skip everything that needs the network
  --only <checks>    comma-separated subset of: ";

/// Every check, in report order.
const CHECKS: &[&str] = &[
    "lockfile",
    "updates",
    "advisories",
    "unused",
    "npm-lockfile",
    "npm-outdated",
    "npm-audit",
    "links",
    "actions",
    "suppressions",
    "notes",
];

const NPMJS_HOST: &str = "registry.npmjs.org";

/// Marks a note to a future run: a workaround tied to nearby code that waits on
/// a condition worth rechecking. See "Housekeeping notes" in CLAUDE.md.
const NOTE_MARKER: &str = "HOUSEKEEPING:";

const CRATES_IO_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// Versions younger than this get flagged: most malicious releases are caught
/// and yanked within days, so a short wait buys a lot.
const MIN_AGE_DAYS: i64 = 7;

/// Longest tool output kept in the report; the rest is reproducible locally.
const MAX_OUTPUT_LINES: usize = 300;

/// Is this variable kept out of child processes? Credentials, plus the
/// per-package variables `cargo run` sets for xtask itself, which aren't meant
/// for other tools: cargo-machete misparses its arguments if it sees
/// CARGO_PKG_NAME.
fn is_withheld_env(key: &str) -> bool {
    matches!(key, "GH_TOKEN" | "GITHUB_TOKEN")
        || key.starts_with("BRAID_")
        || key.starts_with("CARGO_PKG_")
        || key.starts_with("CARGO_MANIFEST_")
}

#[derive(Debug, PartialEq)]
struct Options {
    base: String,
    offline: bool,
    only: Vec<String>,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        base: "origin/main".to_string(),
        offline: false,
        only: CHECKS.iter().map(|c| c.to_string()).collect(),
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--offline" => options.offline = true,
            "--base" => {
                options.base = args.next().ok_or("--base needs a ref")?.clone();
            }
            "--only" => {
                let list = args.next().ok_or("--only needs a list of checks")?;
                let requested: Vec<&str> = list.split(',').collect();
                if let Some(bad) = requested.iter().find(|c| !CHECKS.contains(c)) {
                    return Err(format!("unknown check {bad:?}"));
                }
                // Keep report order regardless of the order requested.
                options.only = CHECKS
                    .iter()
                    .filter(|c| requested.contains(c))
                    .map(|c| c.to_string())
                    .collect();
            }
            other => return Err(format!("unexpected argument {other:?}")),
        }
    }
    Ok(options)
}

/// Entry point; returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    let options = match parse_args(args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("xtask housekeeping: {message}\n{USAGE}{}", CHECKS.join(", "));
            return 2;
        }
    };
    let root = match repo_root() {
        Ok(root) => root,
        Err(message) => {
            eprintln!("xtask housekeeping: {message}");
            return 1;
        }
    };

    let mut results = Vec::new();
    for check in &options.only {
        eprintln!("xtask: housekeeping: {check}");
        results.push((check.as_str(), run_check(check, &root, &options)));
    }

    let report = render_report(&options, &results);
    print!("{}", scrub_paths(&report, &root, std::env::var("HOME").ok().as_deref()));

    if results.iter().any(|(_, outcome)| outcome.status == Status::Error) { 1 } else { 0 }
}

fn run_check(check: &str, root: &Path, options: &Options) -> Outcome {
    match check {
        "lockfile" => check_lockfile(root, options),
        "updates" => check_updates(root, options),
        "advisories" => check_advisories(root, options),
        "unused" => check_unused(root),
        "npm-lockfile" => check_npm_lockfile(root, options),
        "npm-outdated" => check_npm(root, options, "outdated"),
        "npm-audit" => check_npm(root, options, "audit"),
        "links" => check_links(root, options),
        "actions" => check_actions(root),
        "suppressions" => check_suppressions(root),
        "notes" => check_notes(root),
        other => unreachable!("parse_args validated {other}"),
    }
}

// ---------------------------------------------------------------------------
// outcomes and the report
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Status {
    /// Ran; nothing to report.
    Clean,
    /// Ran; something needs a decision.
    Findings,
    /// Ran; an inventory for context, nothing wrong as such.
    Info,
    /// Couldn't run for an expected reason (missing tool, `--offline`).
    Skipped,
    /// Tried to run and failed. Makes the whole command exit 1.
    Error,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Clean => "clean",
            Status::Findings => "findings",
            Status::Info => "info",
            Status::Skipped => "skipped",
            Status::Error => "error",
        }
    }
}

struct Outcome {
    status: Status,
    /// One line for the summary table.
    summary: String,
    /// Markdown for the check's own section; may be empty.
    details: String,
}

impl Outcome {
    fn new(status: Status, summary: impl Into<String>, details: impl Into<String>) -> Self {
        Outcome { status, summary: summary.into(), details: details.into() }
    }
    fn clean() -> Self {
        Self::new(Status::Clean, "", "")
    }
    fn skipped(why: impl Into<String>) -> Self {
        Self::new(Status::Skipped, why, "")
    }
    fn error(why: impl Into<String>, output: &str) -> Self {
        Self::new(Status::Error, why, fenced(output))
    }
}

fn render_report(options: &Options, results: &[(&str, Outcome)]) -> String {
    let mut out = String::from("# Housekeeping report\n\n");
    out.push_str(&format!(
        "Generated by `cargo xtask housekeeping`. Base: `{}`. Network: {}.\n\n",
        options.base,
        if options.offline { "off" } else { "on" },
    ));
    out.push_str("| Check | Status | Summary |\n|---|---|---|\n");
    for (check, outcome) in results {
        // A pipe would end the table cell early.
        let summary = outcome.summary.replace('|', "\\|");
        out.push_str(&format!("| {check} | {} | {summary} |\n", outcome.status.label()));
    }
    for (check, outcome) in results {
        if !outcome.details.is_empty() {
            out.push_str(&format!("\n## {check}\n\n{}\n", outcome.details.trim_end()));
        }
    }
    out
}

/// Fence tool output with a run of backticks longer than any inside it, so the
/// output can't close the fence and turn into live Markdown in a PR.
fn fenced(text: &str) -> String {
    let text = text.trim_end();
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}\n{text}\n{fence}\n")
}

fn truncated(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= MAX_OUTPUT_LINES {
        return text.to_string();
    }
    format!(
        "{}\n... {} more lines cut",
        lines[..MAX_OUTPUT_LINES].join("\n"),
        lines.len() - MAX_OUTPUT_LINES
    )
}

fn scrub_paths(report: &str, root: &Path, home: Option<&str>) -> String {
    let mut out = report.replace(root.to_string_lossy().as_ref(), ".");
    if let Some(home) = home.filter(|h| !h.is_empty()) {
        out = out.replace(home, "~");
    }
    out
}

// ---------------------------------------------------------------------------
// running tools
// ---------------------------------------------------------------------------

struct ToolOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl ToolOutput {
    fn combined(&self) -> String {
        format!("{}\n{}", self.stdout.trim_end(), self.stderr.trim_end()).trim().to_string()
    }
}

/// Run a tool without the environment in `is_withheld_env`. `Err` means it
/// couldn't be started, normally because it isn't installed.
fn tool(program: &str, args: &[&str], dir: &Path) -> std::io::Result<ToolOutput> {
    let mut command = Command::new(program);
    command.args(args).current_dir(dir);
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(is_withheld_env) {
            command.env_remove(key);
        }
    }
    let output = command.output()?;
    Ok(ToolOutput {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn repo_root() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("no current directory: {e}"))?;
    let out = tool("git", &["rev-parse", "--show-toplevel"], &cwd)
        .map_err(|e| format!("cannot run git: {e}"))?;
    if out.code != Some(0) {
        return Err("not inside a git repository".to_string());
    }
    Ok(PathBuf::from(out.stdout.trim()))
}

/// Is `cargo <subcommand>` installed? cargo itself missing counts as no.
fn has_cargo_subcommand(subcommand: &str, root: &Path) -> bool {
    tool("cargo", &[subcommand, "--version"], root).is_ok_and(|o| o.code == Some(0))
}

// ---------------------------------------------------------------------------
// lockfile
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct Package {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    /// npm only: the package runs a script when installed.
    install_script: bool,
}

/// Parse the `[[package]]` tables of a Cargo.lock. Its format is regular
/// enough that this doesn't warrant giving up xtask's no-dependencies rule.
fn parse_lockfile(text: &str) -> Vec<Package> {
    let mut packages = Vec::new();
    let mut current: Option<Package> = None;
    for line in text.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            packages.extend(current.take());
            current = Some(Package {
                name: String::new(),
                version: String::new(),
                source: None,
                checksum: None,
                install_script: false,
            });
            continue;
        }
        if line.starts_with('[') && !line.starts_with("[[package") {
            // e.g. [metadata] in older lockfiles
            packages.extend(current.take());
            continue;
        }
        let (Some(package), Some((key, value))) = (current.as_mut(), line.split_once(" = ")) else {
            continue;
        };
        let value = value.trim_matches('"').to_string();
        match key {
            "name" => package.name = value,
            "version" => package.version = value,
            "source" => package.source = Some(value),
            "checksum" => package.checksum = Some(value),
            _ => {}
        }
    }
    packages.extend(current);
    packages
}

#[derive(Debug, PartialEq)]
enum Change {
    Bumped {
        from: Package,
        to: Package,
    },
    Added(Package),
    Removed(Package),
    /// Same name, version and source, different content.
    ChecksumChanged(Package),
}

impl Change {
    /// The package now in the lockfile, if any.
    fn current(&self) -> Option<&Package> {
        match self {
            Change::Bumped { to, .. } => Some(to),
            Change::Added(p) | Change::ChecksumChanged(p) => Some(p),
            Change::Removed(_) => None,
        }
    }

    fn describe(&self) -> String {
        match self {
            Change::Bumped { from, to } => {
                format!("`{}` {} → {}", to.name, from.version, to.version)
            }
            Change::Added(p) => format!("`{}` {} (new)", p.name, p.version),
            Change::Removed(p) => format!("`{}` {} (removed)", p.name, p.version),
            Change::ChecksumChanged(p) => format!("`{}` {}", p.name, p.version),
        }
    }
}

/// Third-party changes between two lockfiles. Workspace members have no
/// source, and their version bumps aren't dependency changes.
fn lockfile_delta(base: &[Package], current: &[Package]) -> Vec<Change> {
    fn by_name(packages: &[Package]) -> BTreeMap<&str, Vec<&Package>> {
        let mut map: BTreeMap<&str, Vec<&Package>> = BTreeMap::new();
        for package in packages.iter().filter(|p| p.source.is_some()) {
            map.entry(package.name.as_str()).or_default().push(package);
        }
        map
    }
    let base = by_name(base);
    let current = by_name(current);
    let names: BTreeSet<&str> = base.keys().chain(current.keys()).copied().collect();

    let mut changes = Vec::new();
    for name in names {
        let was = base.get(name).map(Vec::as_slice).unwrap_or_default();
        let now = current.get(name).map(Vec::as_slice).unwrap_or_default();
        let same = |a: &Package, b: &Package| a.version == b.version && a.source == b.source;

        for package in now {
            if was.iter().any(|old| same(old, package) && old.checksum != package.checksum) {
                changes.push(Change::ChecksumChanged((*package).clone()));
            }
        }
        let gone: Vec<&&Package> = was.iter().filter(|p| !now.iter().any(|n| same(p, n))).collect();
        let new: Vec<&&Package> = now.iter().filter(|p| !was.iter().any(|o| same(p, o))).collect();
        for pair in 0..gone.len().max(new.len()) {
            changes.push(match (gone.get(pair), new.get(pair)) {
                (Some(from), Some(to)) => {
                    Change::Bumped { from: (**from).clone(), to: (**to).clone() }
                }
                (Some(from), None) => Change::Removed((**from).clone()),
                (None, Some(to)) => Change::Added((**to).clone()),
                (None, None) => unreachable!("pair is below at least one length"),
            });
        }
    }
    changes
}

/// Parse the `packages` of an npm package-lock.json. npm's fixed layout puts
/// entries at indent 4 and their own fields at indent 6, which is regular enough
/// to read without a JSON parser (see `parse_lockfile`). `name` is the path after
/// the first `node_modules/`, so nested copies stay distinct. Entries with no
/// `resolved` URL (workspace links, bundled copies) are left out.
fn parse_npm_lockfile(text: &str) -> Vec<Package> {
    let mut packages = Vec::new();
    let mut current: Option<Package> = None;
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        let line = line.trim();
        if indent == 4 {
            packages.extend(current.take().filter(|p: &Package| p.source.is_some()));
            current = line
                .strip_prefix("\"node_modules/")
                .and_then(|rest| rest.strip_suffix("\": {"))
                .map(|name| Package {
                    name: name.to_string(),
                    version: String::new(),
                    source: None,
                    checksum: None,
                    install_script: false,
                });
            continue;
        }
        let (Some(package), 6) = (current.as_mut(), indent) else { continue };
        let Some((key, value)) = line.trim_end_matches(',').split_once(": ") else { continue };
        let value = value.trim_matches('"');
        match key {
            "\"version\"" => package.version = value.to_string(),
            "\"integrity\"" => package.checksum = Some(value.to_string()),
            "\"hasInstallScript\"" => package.install_script = value == "true",
            "\"resolved\"" => {
                let host = value.split("://").nth(1).and_then(|rest| rest.split('/').next());
                package.source = Some(host.unwrap_or(value).to_string());
            }
            _ => {}
        }
    }
    packages.extend(current.filter(|p: &Package| p.source.is_some()));
    packages
}

/// What a crate's published source says about code it runs at build time.
fn inspect_registry_crate(cargo_home: &Path, name: &str, version: &str) -> Vec<&'static str> {
    let wanted = format!("{name}-{version}");
    let dir = std::fs::read_dir(cargo_home.join("registry/src"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|index| index.path().join(&wanted))
        .find(|dir| dir.is_dir());
    let Some(dir) = dir else {
        return vec!["not inspected (not in registry cache)"];
    };

    let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
    let mut flags = Vec::new();
    // Published manifests write `build = false` when there's no build script.
    if dir.join("build.rs").exists()
        || manifest.lines().any(|line| line.trim_start().starts_with("build = \""))
    {
        flags.push("build script");
    }
    if manifest.lines().any(|line| line.trim() == "proc-macro = true") {
        flags.push("proc macro");
    }
    flags
}

fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))
}

/// When crates.io says this version was published, as an age in whole days.
fn published_age_days(name: &str, version: &str, root: &Path, now: SystemTime) -> Option<i64> {
    let url = format!("https://crates.io/api/v1/crates/{name}/{version}");
    // crates.io's crawler policy asks for an identifying User-Agent.
    let agent = "braid-xtask-housekeeping (github.com/cscheid/braid)";
    let out = tool("curl", &["-sfL", "-A", agent, &url], root).ok()?;
    if out.code != Some(0) {
        return None;
    }
    let created = json_string_field(&out.stdout, "created_at")?;
    age_in_days(&created, now)
}

/// First value of a string field. Good enough for this one response shape,
/// whose first `created_at` is the version's own.
fn json_string_field(json: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\":\"");
    let start = json.find(&needle)? + needle.len();
    let rest = &json[start..];
    Some(rest[..rest.find('"')?].to_string())
}

/// Whole days from an ISO-8601 timestamp to `now`; negative if in the future.
fn age_in_days(timestamp: &str, now: SystemTime) -> Option<i64> {
    let mut date = timestamp.get(..10)?.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: i64 = date.next()?.parse().ok()?;
    let day: i64 = date.next()?.parse().ok()?;
    let today = (now.duration_since(UNIX_EPOCH).ok()?.as_secs() / 86_400) as i64;
    Some(today - days_from_civil(year, month, day))
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn check_lockfile(root: &Path, options: &Options) -> Outcome {
    let current = match std::fs::read_to_string(root.join("Cargo.lock")) {
        Ok(text) => text,
        Err(e) => return Outcome::error("cannot read Cargo.lock", &e.to_string()),
    };
    let spec = format!("{}:Cargo.lock", options.base);
    let base = match tool("git", &["show", &spec], root) {
        Ok(out) if out.code == Some(0) => out.stdout,
        Ok(out) => return Outcome::error(format!("cannot read {spec}"), &out.combined()),
        Err(e) => return Outcome::error("cannot run git", &e.to_string()),
    };

    let changes = lockfile_delta(&parse_lockfile(&base), &parse_lockfile(&current));
    if changes.is_empty() {
        return Outcome::clean();
    }

    let mut notes = Vec::new();
    if !options.offline && changes.iter().any(|c| c.current().is_some()) {
        // Only downloads: cargo fetch doesn't compile or run anything.
        match tool("cargo", &["fetch", "--locked"], root) {
            Ok(out) if out.code == Some(0) => {}
            _ => notes.push("`cargo fetch` failed; some crates may not be inspected.".to_string()),
        }
    }

    let home = cargo_home();
    let now = SystemTime::now();
    let mut flagged = 0;
    let mut queried_crates_io = false;
    let mut rows = String::new();
    for change in &changes {
        let mut flags: Vec<String> = Vec::new();
        if matches!(change, Change::ChecksumChanged(_)) {
            flags.push("checksum changed".to_string());
        }
        if let Some(package) = change.current() {
            if package.source.as_deref() == Some(CRATES_IO_SOURCE) {
                if let Some(home) = &home {
                    let found = inspect_registry_crate(home, &package.name, &package.version);
                    flags.extend(found.iter().map(|f| f.to_string()));
                }
                if !options.offline {
                    // crates.io's crawler policy: at most one request a second.
                    if queried_crates_io {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    queried_crates_io = true;
                    match published_age_days(&package.name, &package.version, root, now) {
                        Some(age) if age < MIN_AGE_DAYS => {
                            flags.push(format!("published under {MIN_AGE_DAYS} days ago"));
                        }
                        Some(_) => {}
                        None => flags.push("publish date unknown".to_string()),
                    }
                }
            } else {
                flags.push("non-crates.io source".to_string());
            }
        }
        if !flags.is_empty() {
            flagged += 1;
        }
        rows.push_str(&format!("- {} {}\n", change.describe(), render_flags(&flags)));
    }

    let mut details = notes.join("\n");
    if !details.is_empty() {
        details.push_str("\n\n");
    }
    details.push_str(&rows);
    Outcome::new(
        Status::Findings,
        format!("{} crate(s) changed, {flagged} flagged", changes.len()),
        details,
    )
}

/// When npm says this version was published, as an age in whole days. A registry
/// document's `time` map is the only place a version is a key with a string value.
fn npm_published_age_days(path: &str, version: &str, root: &Path, now: SystemTime) -> Option<i64> {
    let name = path.rsplit("node_modules/").next()?;
    let url = format!("https://{NPMJS_HOST}/{}", name.replace('/', "%2f"));
    let agent = "braid-xtask-housekeeping (github.com/cscheid/braid)";
    let out = tool("curl", &["-sfL", "-A", agent, &url], root).ok()?;
    if out.code != Some(0) {
        return None;
    }
    age_in_days(&json_string_field(&out.stdout, version)?, now)
}

fn check_npm_lockfile(root: &Path, options: &Options) -> Outcome {
    let Ok(current) = std::fs::read_to_string(root.join("ui/package-lock.json")) else {
        return Outcome::skipped("no ui/package-lock.json");
    };
    let spec = format!("{}:ui/package-lock.json", options.base);
    let base = match tool("git", &["show", &spec], root) {
        Ok(out) if out.code == Some(0) => out.stdout,
        Ok(out) => return Outcome::error(format!("cannot read {spec}"), &out.combined()),
        Err(e) => return Outcome::error("cannot run git", &e.to_string()),
    };

    let changes = lockfile_delta(&parse_npm_lockfile(&base), &parse_npm_lockfile(&current));
    if changes.is_empty() {
        return Outcome::clean();
    }

    let now = SystemTime::now();
    let mut flagged = 0;
    let mut rows = String::new();
    for change in &changes {
        let mut flags: Vec<String> = Vec::new();
        if matches!(change, Change::ChecksumChanged(_)) {
            flags.push("checksum changed".to_string());
        }
        if let Some(package) = change.current() {
            if package.install_script {
                flags.push("install script".to_string());
            }
            if package.source.as_deref() != Some(NPMJS_HOST) {
                flags.push("non-npmjs source".to_string());
            } else if !options.offline {
                match npm_published_age_days(&package.name, &package.version, root, now) {
                    Some(age) if age < MIN_AGE_DAYS => {
                        flags.push(format!("published under {MIN_AGE_DAYS} days ago"));
                    }
                    Some(_) => {}
                    None => flags.push("publish date unknown".to_string()),
                }
            }
        }
        if !flags.is_empty() {
            flagged += 1;
        }
        rows.push_str(&format!("- {} {}\n", change.describe(), render_flags(&flags)));
    }
    Outcome::new(
        Status::Findings,
        format!("{} package(s) changed, {flagged} flagged", changes.len()),
        rows,
    )
}

fn render_flags(flags: &[String]) -> String {
    if flags.is_empty() { String::new() } else { format!("**[{}]**", flags.join("; ")) }
}

// ---------------------------------------------------------------------------
// tool-backed checks
// ---------------------------------------------------------------------------

/// The actionable lines of `cargo update --dry-run --verbose`.
fn update_lines(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.contains(" -> ")
                || line.contains("(available:")
                || line.starts_with("Adding ")
                || line.starts_with("Removing ")
        })
        .collect()
}

fn check_updates(root: &Path, options: &Options) -> Outcome {
    if options.offline {
        return Outcome::skipped("needs the network (--offline)");
    }
    let out = match tool("cargo", &["update", "--dry-run", "--verbose"], root) {
        Ok(out) => out,
        Err(_) => return Outcome::skipped("cargo not found"),
    };
    if out.code != Some(0) {
        return Outcome::error("cargo update --dry-run failed", &out.combined());
    }
    let lines = update_lines(&out.stderr);
    if lines.is_empty() {
        return Outcome::clean();
    }
    let held = lines.iter().filter(|line| line.contains("(available:")).count();
    Outcome::new(
        Status::Findings,
        format!("{} change(s) available, {held} held back", lines.len() - held),
        fenced(&truncated(&lines.join("\n"))),
    )
}

fn check_advisories(root: &Path, options: &Options) -> Outcome {
    if !has_cargo_subcommand("audit", root) {
        return Outcome::skipped("needs `cargo install cargo-audit`");
    }
    let args: &[&str] = if options.offline { &["audit", "--no-fetch"] } else { &["audit"] };
    let out = match tool("cargo", args, root) {
        Ok(out) => out,
        Err(e) => return Outcome::error("cannot run cargo audit", &e.to_string()),
    };
    // cargo-audit exits 1 for both vulnerabilities and its own failures, and 0
    // when it only has warnings (e.g. unmaintained), so go by what it printed.
    if out.stdout.contains("Crate:") {
        let entries = out.stdout.lines().filter(|line| line.starts_with("Crate:")).count();
        return Outcome::new(
            Status::Findings,
            format!("{entries} advisory entries"),
            fenced(&truncated(out.stdout.trim_end())),
        );
    }
    if out.code == Some(0) {
        Outcome::clean()
    } else {
        Outcome::error("cargo audit failed", &out.combined())
    }
}

fn check_unused(root: &Path) -> Outcome {
    if !has_cargo_subcommand("machete", root) {
        return Outcome::skipped("needs `cargo install cargo-machete`");
    }
    // Scoped to crates/: gitignored external-sources/ holds other projects'
    // manifests.
    let out = match tool("cargo", &["machete", "crates"], root) {
        Ok(out) => out,
        Err(e) => return Outcome::error("cannot run cargo machete", &e.to_string()),
    };
    match out.code {
        Some(0) => Outcome::clean(),
        Some(1) => Outcome::new(
            Status::Findings,
            "possibly unused dependencies",
            fenced(&truncated(&out.combined())),
        ),
        _ => Outcome::error("cargo machete failed", &out.combined()),
    }
}

fn check_npm(root: &Path, options: &Options, subcommand: &str) -> Outcome {
    if options.offline {
        return Outcome::skipped("needs the network (--offline)");
    }
    let ui = root.join("ui");
    if !ui.join("package.json").is_file() {
        return Outcome::skipped("no ui/package.json");
    }
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let out = match tool(npm, &[subcommand], &ui) {
        Ok(out) => out,
        Err(_) => return Outcome::skipped("needs Node.js / npm"),
    };
    // Both subcommands exit 1 for findings as well as failures. Findings go
    // to stdout, so a bare exit 1 is a failure (e.g. no network).
    if out.code == Some(0) {
        Outcome::clean()
    } else if out.stdout.trim().is_empty() {
        Outcome::error(format!("npm {subcommand} failed"), &out.combined())
    } else {
        let summary =
            if subcommand == "audit" { "vulnerabilities reported" } else { "packages outdated" };
        Outcome::new(Status::Findings, summary, fenced(&truncated(out.stdout.trim_end())))
    }
}

fn check_links(root: &Path, options: &Options) -> Outcome {
    let present = tool("lychee", &["--version"], root).is_ok_and(|o| o.code == Some(0));
    if !present {
        return Outcome::skipped("needs lychee (`cargo install lychee` or `brew install lychee`)");
    }
    let mut args = vec!["--no-progress"];
    if options.offline {
        args.push("--offline");
    }
    let targets = ["README.md", "CLAUDE.md", "docs", "crates/braid/src/agents-info.md"];
    args.extend(targets.iter().filter(|t| root.join(t).exists()));

    let out = match tool("lychee", &args, root) {
        Ok(out) => out,
        Err(e) => return Outcome::error("cannot run lychee", &e.to_string()),
    };
    // lychee: 0 all good, 2 broken links, anything else is its own failure.
    match out.code {
        Some(0) => Outcome::clean(),
        Some(2) => {
            Outcome::new(Status::Findings, "broken links", fenced(&truncated(&out.combined())))
        }
        _ => Outcome::error("lychee failed", &out.combined()),
    }
}

// ---------------------------------------------------------------------------
// repo inventories
// ---------------------------------------------------------------------------

/// The action reference on a workflow line, if it has one. Local actions
/// (`./path`) have no version, so they're left out.
fn parse_uses(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let line = line.strip_prefix("- ").unwrap_or(line).trim_start();
    let value = line.strip_prefix("uses:")?;
    let value = value.split(" #").next()?.trim().trim_matches(|c| c == '"' || c == '\'');
    if value.is_empty() || value.starts_with('.') { None } else { Some(value) }
}

fn check_actions(root: &Path) -> Outcome {
    let dir = root.join(".github/workflows");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Outcome::skipped("no .github/workflows");
    };
    // action name -> version -> number of uses
    let mut actions: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|e| e == "yml" || e == "yaml") {
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                return Outcome::error(format!("cannot read {}", path.display()), &e.to_string());
            }
        };
        for reference in text.lines().filter_map(parse_uses) {
            let (name, version) = reference.split_once('@').unwrap_or((reference, "(unpinned)"));
            *actions.entry(name.into()).or_default().entry(version.into()).or_default() += 1;
        }
    }
    if actions.is_empty() {
        return Outcome::clean();
    }

    let mut mixed = String::new();
    let mut inventory = String::new();
    let mut mixed_count = 0;
    for (name, versions) in &actions {
        let list: Vec<&str> = versions.keys().map(String::as_str).collect();
        if list.len() > 1 {
            mixed_count += 1;
            mixed.push_str(&format!("- `{name}`: mixed versions {}\n", list.join(", ")));
        }
        for (version, count) in versions {
            inventory.push_str(&format!("- `{name}@{version}` ×{count}\n"));
        }
    }
    let status = if mixed_count > 0 { Status::Findings } else { Status::Info };
    let summary = format!("{} action(s), {mixed_count} with mixed versions", actions.len());
    let details =
        if mixed.is_empty() { inventory } else { format!("{mixed}\nAll:\n\n{inventory}") };
    Outcome::new(status, summary, details)
}

fn check_suppressions(root: &Path) -> Outcome {
    const MARKERS: &[&str] = &["#[allow(", "#![allow(", "#[ignore", "TODO", "FIXME"];
    // These list the markers themselves.
    const EXEMPT: &[&str] =
        &["crates/xtask/src/housekeeping.rs", "crates/xtask/tests/housekeeping.rs"];

    let files = match tool("git", &["ls-files", "--", "*.rs"], root) {
        Ok(out) if out.code == Some(0) => out.stdout,
        Ok(out) => return Outcome::error("git ls-files failed", &out.combined()),
        Err(e) => return Outcome::error("cannot run git", &e.to_string()),
    };
    let mut hits = Vec::new();
    for file in files.lines().filter(|file| !EXEMPT.contains(file)) {
        let Ok(text) = std::fs::read_to_string(root.join(file)) else { continue };
        for (index, line) in text.lines().enumerate() {
            if MARKERS.iter().any(|marker| line.contains(marker)) {
                hits.push(format!("{file}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    if hits.is_empty() {
        return Outcome::clean();
    }
    Outcome::new(
        Status::Info,
        format!("{} occurrence(s) to reconsider", hits.len()),
        fenced(&truncated(&hits.join("\n"))),
    )
}

fn check_notes(root: &Path) -> Outcome {
    // These define the marker rather than use it.
    const EXEMPT: &[&str] =
        &["crates/xtask/src/housekeeping.rs", "crates/xtask/tests/housekeeping.rs"];

    let files = match tool("git", &["ls-files"], root) {
        Ok(out) if out.code == Some(0) => out.stdout,
        Ok(out) => return Outcome::error("git ls-files failed", &out.combined()),
        Err(e) => return Outcome::error("cannot run git", &e.to_string()),
    };
    let mut hits = Vec::new();
    // Markdown is left out: notes belong beside code, and docs describe the marker.
    for file in files.lines().filter(|f| !f.ends_with(".md") && !EXEMPT.contains(f)) {
        // Binary files fail to read as text, and can't hold a note anyway.
        let Ok(text) = std::fs::read_to_string(root.join(file)) else { continue };
        for (index, line) in text.lines().enumerate() {
            if line.contains(NOTE_MARKER) {
                hits.push(format!("{file}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    if hits.is_empty() {
        return Outcome::clean();
    }
    Outcome::new(
        Status::Info,
        format!("{} note(s) to recheck", hits.len()),
        fenced(&truncated(&hits.join("\n"))),
    )
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, version: &str, checksum: &str) -> Package {
        Package {
            name: name.into(),
            version: version.into(),
            source: Some(CRATES_IO_SOURCE.into()),
            checksum: Some(checksum.into()),
            install_script: false,
        }
    }

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn parses_lockfile_packages() {
        let text = "version = 4\n\n[[package]]\nname = \"local\"\nversion = \"0.1.0\"\n\
                    dependencies = [\n \"dep\",\n]\n\n[[package]]\nname = \"dep\"\n\
                    version = \"1.2.3\"\nsource = \"registry+x\"\nchecksum = \"abc\"\n";
        let packages = parse_lockfile(text);
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "local");
        assert_eq!(packages[0].source, None);
        assert_eq!(packages[1].version, "1.2.3");
        assert_eq!(packages[1].checksum.as_deref(), Some("abc"));
    }

    #[test]
    fn lockfile_parser_stops_at_other_tables() {
        let text = "[[package]]\nname = \"a\"\nversion = \"1\"\n\n[metadata]\nname = \"nope\"\n";
        assert_eq!(parse_lockfile(text)[0].name, "a");
    }

    #[test]
    fn delta_handles_two_versions_of_one_crate() {
        let base = [package("syn", "1.0.0", "a"), package("syn", "2.0.0", "b")];
        // syn 1 stays, syn 2 moves: a naive by-name diff would call this 1 -> 2.1.
        let current = [package("syn", "1.0.0", "a"), package("syn", "2.1.0", "c")];
        let delta = lockfile_delta(&base, &current);
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].describe(), "`syn` 2.0.0 → 2.1.0");
    }

    #[test]
    fn delta_reports_a_newly_duplicated_crate_as_added() {
        let base = [package("syn", "2.0.0", "a")];
        let current = [package("syn", "1.0.0", "b"), package("syn", "2.0.0", "a")];
        assert_eq!(lockfile_delta(&base, &current), [Change::Added(package("syn", "1.0.0", "b"))]);
    }

    #[test]
    fn age_counts_days_across_month_and_leap_boundaries() {
        let at = |days: u64| UNIX_EPOCH + Duration::from_secs(days * 86_400);
        assert_eq!(age_in_days("1970-01-01T00:00:00Z", at(0)), Some(0));
        assert_eq!(age_in_days("1970-01-01T23:00:00Z", at(10)), Some(10));
        // 2024-03-01 is day 19783, and 2024 is a leap year.
        assert_eq!(age_in_days("2024-02-28T00:00:00Z", at(19_783)), Some(2));
        assert_eq!(age_in_days("2024-03-02T00:00:00Z", at(19_783)), Some(-1));
    }

    #[test]
    fn age_rejects_malformed_timestamps() {
        assert_eq!(age_in_days("soon", UNIX_EPOCH), None);
        assert_eq!(age_in_days("yyyy-mm-ddT00", UNIX_EPOCH), None);
    }

    #[test]
    fn reads_string_fields_from_json() {
        let json = r#"{"version":{"num":"1","created_at":"2026-01-02T00:00:00Z","x":1}}"#;
        assert_eq!(json_string_field(json, "created_at").unwrap(), "2026-01-02T00:00:00Z");
        assert_eq!(json_string_field(json, "missing"), None);
    }

    #[test]
    fn fence_outgrows_backticks_in_the_text() {
        assert_eq!(fenced("plain"), "```\nplain\n```\n");
        assert_eq!(fenced("a ```` b"), "`````\na ```` b\n`````\n");
    }

    #[test]
    fn truncation_keeps_short_output_and_says_when_it_cuts() {
        assert_eq!(truncated("a\nb"), "a\nb");
        let long = vec!["x"; MAX_OUTPUT_LINES + 5].join("\n");
        assert!(truncated(&long).ends_with("... 5 more lines cut"));
    }

    #[test]
    fn scrubbing_handles_an_empty_home() {
        let scrubbed = scrub_paths("/r/a and /h/b", Path::new("/r"), Some(""));
        assert_eq!(scrubbed, "./a and /h/b");
        assert_eq!(scrub_paths("/r/a and /h/b", Path::new("/r"), Some("/h")), "./a and ~/b");
    }

    #[test]
    fn uses_parsing() {
        assert_eq!(parse_uses("      - uses: actions/checkout@v6"), Some("actions/checkout@v6"));
        assert_eq!(parse_uses("    uses: a/b@v1 # why"), Some("a/b@v1"));
        assert_eq!(parse_uses("  - uses: 'a/b@v1'"), Some("a/b@v1"));
        assert_eq!(parse_uses("  - uses: ./local"), None);
        assert_eq!(parse_uses("  - run: echo uses: a/b@v1"), None);
    }

    #[test]
    fn default_options_run_every_check_against_origin_main() {
        let options = parse_args(&[]).unwrap();
        assert_eq!(options.base, "origin/main");
        assert!(!options.offline);
        assert_eq!(options.only.len(), CHECKS.len());
    }

    #[test]
    fn only_keeps_report_order() {
        let options = parse_args(&strings(&["--only", "links,lockfile"])).unwrap();
        assert_eq!(options.only, ["lockfile", "links"]);
    }

    #[test]
    fn flags_with_missing_values_are_errors() {
        assert!(parse_args(&strings(&["--base"])).is_err());
        assert!(parse_args(&strings(&["--only"])).is_err());
    }
}
