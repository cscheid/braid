# rustfmt enforcement + cargo xtask pre-push pipeline

Strand: **br-xtask-fmt-jeuvc2d0** (P3 chore, labels: tooling, ci).

## Overview

The repo is hand-formatted: `cargo fmt --check` flags 223 sites under
default config, 109 under `use_small_heuristics = "Max"` (the closest
stable config to the house style — compact struct literals, dense
one-liners; `max_width` already defaults to 100). Enforcement therefore
needs: a `rustfmt.toml`, a one-time mechanical reformat commit, a CI
gate, and a local "get ready to push" entry point.

Mechanism (decided with Carlos, 2026-06-04): **cargo xtask** — a
workspace member binary + `.cargo/config.toml` alias, the de-facto
cargo-idiomatic repo-automation pattern. No external tools, logic in
plain Rust. The CI gate is the binding enforcement (hooks are per-clone
and `--no-verify`-skippable); the optional pre-push hook is convenience.

## Design decisions

1. **rustfmt config**: `use_small_heuristics = "Max"` only. All-stable
   options; no nightly toolchain requirement.
2. **Crate location**: `crates/xtask` (matches this repo's layout; the
   rust-analyzer convention of top-level `xtask/` fits repos without a
   `crates/` dir). Zero runtime dependencies — std only; subprocesses
   via `std::process::Command`.
3. **Subcommands**:
   - `cargo xtask ci [--dry-run]` — fmt --check → clippy --all-targets
     -D warnings → build --workspace --all-targets → test --workspace.
     Fail-fast cheap-first ordering; build-before-test mirrors ci.yml's
     assert_cmd relink-race avoidance. `--dry-run` prints the command
     list without executing (and is what makes the sequence testable
     without nesting cargo-in-cargo).
   - `cargo xtask fmt` — apply formatting (`cargo fmt --all`).
   - `cargo xtask install-hooks` — write `.git/hooks/pre-push` running
     `cargo xtask ci`. Explicit opt-in only, never automatic.
4. **Hook safety**: resolve the hooks dir via `git rev-parse --git-path
   hooks` (correct under worktrees and `core.hooksPath`); the hook file
   carries a marker line; installing over a foreign (marker-less)
   pre-push refuses with an explanatory error; re-install is idempotent.
5. **ci.yml keeps explicit steps** (better CI log granularity + cache
   interaction) and gains a `cargo fmt --check` step; xtask mirrors the
   sequence with a cross-referencing comment in both places.
6. **Sequencing**: the mechanical reformat is its own commit, separate
   from any semantic change.

## Work items

### Phase 1 — rustfmt.toml + mechanical reformat

- [ ] Add `rustfmt.toml` (`use_small_heuristics = "Max"`), commit
- [ ] `cargo fmt` mechanical commit: verify the diff contains no
      semantic change (tests + clippy green before commit), nothing else
      in the commit

### Phase 2 — xtask crate (tests first)

- [ ] e2e tests (`crates/xtask/tests/cli.rs`, assert_cmd):
      `ci --dry-run` prints the exact 4-command sequence; unknown /
      missing subcommand exits nonzero with usage on stderr;
      `install-hooks` in a scratch git repo creates an executable
      pre-push containing the marker and `cargo xtask ci`; re-run is
      idempotent; a pre-existing foreign pre-push is refused (file
      untouched); works from a subdirectory of the repo
- [ ] Implement `crates/xtask` (std-only), add to workspace members
- [ ] `.cargo/config.toml`: `[alias] xtask = "run -q -p xtask --"`
- [ ] Run `cargo xtask ci` for real once — full pipeline green

### Phase 3 — CI gate

- [ ] ci.yml: add rustfmt component + `cargo fmt --check` step to the
      test job; cross-reference comments between ci.yml and xtask

### Phase 4 — docs & wrap-up

- [ ] CLAUDE.md: run `cargo xtask ci` before asking to push; mention
      `cargo xtask install-hooks`
- [ ] Comment + close strand br-xtask-fmt-jeuvc2d0 (after push)
