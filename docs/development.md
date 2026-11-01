# Development

## Prerequisites

To build and test a checkout, you need:

| Tool | Needed for | Install |
|---|---|---|
| Rust (stable), with clippy and rustfmt | Everything. CI uses the latest stable, and the repo doesn't pin a version, so keep yours current | [rustup](https://rustup.rs), then `rustup update stable` and `rustup component add clippy rustfmt` |
| Node.js 22.12 or newer, with npm | Any `cargo build` or `cargo test`, because `crates/braid/build.rs` builds the web UI, and the UI tests | [nodejs.org](https://nodejs.org) or your version manager |
| minisign | `cargo test`: the installer tests fail if it's missing | `brew install minisign`, `apt install minisign`, `dnf install minisign`, or `apk add minisign` |

The first build also needs network access, to download crates and run `npm ci`.
Setting `SKIP_UI_BUILD=1` lets you build without Node.js, but `braid ui` then
serves a stub page unless `ui/dist/` has already been built.

These are only needed for specific tasks:

| Tool | Needed for | Install |
|---|---|---|
| `tauri-cli` | `cargo xtask viewer-dev`, `cargo xtask viewer-build` | `cargo install tauri-cli --version '^2' --locked`; Linux also needs the [system packages](viewer.md) |
| `mdbook` | `cargo xtask docs`, `cargo xtask docs-serve` | `cargo install mdbook` |
| `cargo-audit` | The housekeeping `advisories` check | `cargo install cargo-audit --locked` |
| `cargo-machete` | The housekeeping `unused` check | `cargo install cargo-machete --locked` |
| `lychee` | The housekeeping `links` check | `cargo install lychee --locked` or `brew install lychee` |

## Building and testing

```sh
cargo xtask ci    # the same pipeline as CI: fmt --check, clippy, build, tests, UI tests
cargo xtask fmt   # apply formatting
cargo test        # just the Rust tests (default members only)
```

Run `cargo xtask ci` before pushing. To have that happen automatically, install
a pre-push hook with `cargo xtask install-hooks`.

The workspace has five crates: `braid-core` (schema, automerge
hydrate/reconcile, ready/blocked logic — no I/O), `braid` (CLI, config
discovery, cache, sync), `braid-config` (lean config/registry shared by
braid and braid-viewer), `xtask` (repo automation, std-only), and
`braid-viewer` (Tauri desktop app, excluded from default builds — use
`cargo xtask viewer-*` or `-p braid-viewer`).
Design decisions and phase history live in
`claude-notes/plans/2026/06/03/braid-design-kickoff.md`; vocabulary in
[terminology](terminology.md). This repo dogfoods braid — run `braid list`
here to see its own skein.

## Housekeeping report

`cargo xtask housekeeping` prints a Markdown report of periodic repo chores. It
never changes the repo, runs no dependency code (no build scripts or proc
macros), and doesn't touch your skein, so it's safe to run on a dependency bump
nobody has reviewed yet.

```sh
cargo xtask housekeeping                     # everything, comparing to origin/main
cargo xtask housekeeping --offline           # skip anything needing the network
cargo xtask housekeeping --only lockfile,links
cargo xtask housekeeping --base <ref>        # compare lockfiles to another ref
```

| Check | Reports | Needs |
|---|---|---|
| `lockfile` | Crates changed since the base ref, flagging build scripts, proc macros, non-crates.io sources, changed checksums, and versions published under 7 days ago | |
| `updates` | Available Cargo updates | network |
| `advisories` | RustSec advisories | `cargo-audit` |
| `unused` | Possibly unused dependencies | `cargo-machete` |
| `npm-lockfile` | `ui/` packages changed since the base ref, flagging install scripts, non-npmjs sources, changed checksums, and versions published under 7 days ago | |
| `npm-outdated`, `npm-audit` | State of `ui/` dependencies | npm, network |
| `links` | Broken links in the docs | `lychee` |
| `actions` | GitHub Actions in use, flagging mixed versions | |
| `suppressions` | `#[allow]`, `#[ignore]`, `TODO` and `FIXME` in Rust files | |
| `notes` | `HOUSEKEEPING:` comments left for future runs, in any tracked file except Markdown | |

Install commands are under [Prerequisites](#prerequisites). A check whose tool is missing shows as `skipped` in the summary rather than
passing. The command exits `0` when every check ran, even with findings or
skips, `1` if a check failed to run, and `2` on a usage error.

## Building these docs

This site is an [mdBook](https://rust-lang.github.io/mdBook/) built from the
`docs/` directory (`book.toml` at the repo root, `docs/SUMMARY.md` is the
table of contents):

```sh
cargo xtask docs            # build to book/
cargo xtask docs-serve      # live-reload preview at http://localhost:3000
```

Every `docs/*.md` page must be listed in `docs/SUMMARY.md` — a drift test
(`crates/braid/tests/docs_drift.rs`) fails the build otherwise. Pushes to
`main` publish the site via `.github/workflows/docs.yml`.
