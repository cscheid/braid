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
