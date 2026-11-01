# Development

```sh
cargo test        # 120+ tests, no network required (default-members only)
cargo clippy --all-targets
```

The workspace has four crates: `braid-core` (schema, automerge
hydrate/reconcile, ready/blocked logic — no I/O), `braid` (CLI, config
discovery, cache, sync), `braid-config` (lean config/registry shared by
braid and braid-viewer), and `braid-viewer` (Tauri desktop app, excluded
from default builds — use `cargo xtask viewer-*` or `-p braid-viewer`).
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
cargo xtask housekeeping --base <ref>        # compare Cargo.lock to another ref
```

| Check | Reports | Needs |
|---|---|---|
| `lockfile` | Crates changed since the base ref, flagging build scripts, proc macros, non-crates.io sources, changed checksums, and versions published under 7 days ago | |
| `updates` | Available Cargo updates | network |
| `advisories` | RustSec advisories | `cargo-audit` |
| `unused` | Possibly unused dependencies | `cargo-machete` |
| `npm-outdated`, `npm-audit` | State of `ui/` dependencies | npm, network |
| `links` | Broken links in the docs | `lychee` |
| `actions` | GitHub Actions in use, flagging mixed versions | |
| `suppressions` | `#[allow]`, `#[ignore]`, `TODO` and `FIXME` in Rust files | |

A check whose tool is missing shows as `skipped` in the summary rather than
passing. The command exits `0` when every check ran, even with findings or
skips, `1` if a check failed to run, and `2` on a usage error.

## Building these docs

This site is an [mdBook](https://rust-lang.github.io/mdBook/) built from the
`docs/` directory (`book.toml` at the repo root, `docs/SUMMARY.md` is the
table of contents):

```sh
cargo install mdbook        # once
cargo xtask docs            # build to book/
cargo xtask docs-serve      # live-reload preview at http://localhost:3000
```

Every `docs/*.md` page must be listed in `docs/SUMMARY.md` — a drift test
(`crates/braid/tests/docs_drift.rs`) fails the build otherwise. Pushes to
`main` publish the site via `.github/workflows/docs.yml`.
