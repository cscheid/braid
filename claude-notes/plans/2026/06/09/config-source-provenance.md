# Surface config source provenance (br-y6ra26a3)

## Overview

Users can't tell which config layer supplied `doc_id` / `sync_server` /
`author`. The provenance is already computed for `doc_id` (`SecretSource`)
but discarded for the rest, and never surfaced. Generalize source tracking
to all three fields and expose it two ways:

1. a stderr provenance line on `braid secret`
2. a new `braid config` subcommand printing every resolved field, its
   (redacted) value, and where it came from

`doc_id` stays a bearer secret: `braid config` shows only a redacted
prefix (full disclosure remains `braid secret`).

## Design

- Rename `config::SecretSource` → `config::Source`; widen to cover every
  origin: `Env(String)` (carries the var name), `RepoFile(PathBuf)`,
  `UserConfig { project, path }`, `GitConfig`, `OsUser`, `Default`.
- `ConfigInputs.user_config: Option<UserConfig>` → `Option<(PathBuf,
  UserConfig)>` so the resolved user-config path (XDG-honoring) flows into
  `Source::UserConfig`.
- `ResolvedConfig` gains `sync_server_source` and `author_source`
  (alongside the existing, renamed `doc_id_source`).
- `Source::describe()` → human string for CLI output.
- `commands::config(cwd)` prints the three fields; `commands::secret`
  gains a stderr `doc_id resolved from …` line.
- `Cmd::Config` in main.rs.

## Phase 1 — Tests (TDD)

- [x] `tests/config.rs`: assert `sync_server_source` / `author_source` for
      each origin (env, repo file, user config, default/git/os); update the
      existing `doc_id_source` assertions to the new `Source` shape.
- [x] `tests/config.rs`: `Source::describe()` renders each variant.
- [x] `tests/secret_hygiene.rs`: `braid secret` stderr names the source.
- [x] `tests/config_cli.rs` (new): `braid config` shows each field + source,
      redacts the doc id, and never prints the full doc id.
- [x] docs_drift will require `braid config` in agents-info (Phase 3).

## Phase 2 — Implementation

- [x] Widen `Source`, add `describe()`.
- [x] Thread user-config path through `ConfigInputs` / `gather_fs`.
- [x] `resolve` tracks all three sources.
- [x] `commands::config` + `commands::secret` provenance line.
- [x] `Cmd::Config` wiring in main.rs.

## Phase 3 — Docs

- [x] `agents-info.md`: `braid config` row + note in the Setup section.
- [x] README if warranted.
- [x] `cargo xtask ci` green.
