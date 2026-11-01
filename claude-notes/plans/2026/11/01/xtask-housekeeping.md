# `cargo xtask housekeeping`

## Overview

A deterministic, reviewable command that does the directly automatable part
of periodic repo chores and prints a Markdown report. A future
`braid-housekeeping` skill runs it, acts on the judgment calls, and includes
the report in its PR, so the reviewer has facts the agent did not write.

## Invariants

- **Runs no dependency code.** xtask stays std-only, and nothing here invokes
  `cargo build`/`test` or an npm install. Keep it that way: this is what makes
  it safe to run before anyone has looked at a bump.
- **Read-only.** No repo changes and no pushes. `cargo fetch` only fills the
  registry cache; `cargo update` is `--dry-run`.
- **Stays away from secrets.** Does not read `.braid.toml` or invoke `braid`,
  and strips `BRAID_*`, `GH_TOKEN` and `GITHUB_TOKEN` from child processes.
  (Also strips `CARGO_PKG_*`/`CARGO_MANIFEST_*`, for correctness rather than
  secrecy: under `cargo run`, `CARGO_PKG_NAME` breaks cargo-machete's arguments.)
- **The report is safe to publish.** Repo and home paths are scrubbed; tool
  output is fenced so it cannot break out into PR Markdown or @-mention anyone.
- **Never silently passes.** A missing tool is `skipped` with an install hint,
  shown in the summary.

## Exit codes

`0` every check ran (findings and skips are fine), `1` a check errored,
`2` usage error. Findings are the expected output, not a failure.

## Work items

### Phase 1: tests first

- [x] e2e tests with fake tools on `PATH` (`crates/xtask/tests/housekeeping.rs`)
- [x] Unit tests for the pure parts (lockfile, delta, dates, fences, actions)
- [x] Confirm the new tests fail before implementing

### Phase 2: implementation

- [x] `lockfile`: delta against `--base`, with flags
- [x] `updates`: `cargo update --dry-run --verbose`
- [x] `advisories`: `cargo audit`
- [x] `unused`: `cargo machete`
- [x] `npm-outdated`, `npm-audit`
- [x] `links`: `lychee`, offline with `--offline`
- [x] `actions`: inventory of `uses:`, flagging mixed versions
- [x] `suppressions`: `#[allow]`, `#[ignore]`, TODO, FIXME
- [x] `--base`, `--offline`, `--only`

### Phase 3: verify and document

- [x] Run against the real repo, with real `cargo-audit` and `cargo-machete`
- [x] Usage text, crate description, `docs/development.md`
- [x] `cargo xtask ci`

## Out of scope for now

- The skill itself, and the `CLAUDE.md` pushing rule that goes with it
- Skein data for the skill (needs the doc id, so needs its own design)
- Checking whether a pinned GitHub Action has a newer release
- `--json` output
