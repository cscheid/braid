# Richer list filters: --label, --assignee, --type (br-jd16my6w)

## Overview

`braid list` filters by `--status` only; `braid ready` has no filters. Add
`--label` (repeatable, AND semantics), `--assignee`, and `--type` to both
`list` and `ready`. The JSON output shape (array of full Issue objects)
stays unchanged — these flags only narrow which strands appear.

Also extend the MCP `braid_list` and `braid_ready` tools with matching
optional parameters (`labels: string[]`, `assignee: string`, `type:
string`), since braid is agent-centric and MCP parity is cheap here.

## Design decisions

- **Shared filter type.** Introduce a small `ListFilter` (working name)
  struct in `braid-core` (`domain.rs`): `labels: Vec<String>`, `assignee:
  Option<String>`, `issue_type: Option<IssueType>`, with a
  `matches(&Issue) -> bool` method. Both the `list` path
  (`ops.rs::list`) and the `ready` path (post-`ready_issues`) apply it,
  so semantics can't drift between the two commands.
- **Label semantics:** repeatable flag; a strand must carry *all* given
  labels (AND). Exact string match (labels are a `BTreeSet<String>`).
- **Assignee semantics:** exact match on `assignee`. Strands with no
  assignee never match. (No "unassigned" sentinel for now — can be a
  follow-up strand if wanted.)
- **Type semantics:** parse via the same `IssueType` parsing used by
  `create --type` (serde/Display round-trip), so `feature`, `bug`, etc.
  and `Other(...)` strings behave consistently.
- **`--status` on `list` unchanged**; new flags compose with it (and with
  `--all`). `ready` keeps its hardcoded awake+unblocked logic, with the
  new filters applied on top.
- **Filtering happens in core, not in command-layer post-processing**, so
  the MCP tools get it for free through the same `Session` methods.

## Phase 1 — Tests (TDD)

- [x] CLI tests in `crates/braid/tests/cli.rs`:
  - [x] `list --label x` returns only strands with label x
  - [x] `list --label x --label y` requires both (AND)
  - [x] `list --assignee alice` exact-match; unassigned strands excluded
  - [x] `list --type bug` filters by issue type
  - [x] filters compose: `--status` + `--label` + `--type` together
  - [x] `--json` output with filters is still an array of full Issue
        objects (shape unchanged)
- [x] `ready` filter tests in `crates/braid/tests/deps_cli.rs`:
  - [x] `ready --label x` / `--assignee` / `--type` narrow the ready set
        and still exclude blocked/deferred strands
- [x] Unit tests for `ListFilter::matches` in `braid-core` (label AND,
      assignee None handling, type match)
- [x] MCP tests in `crates/braid/tests/mcp_cli.rs`: `braid_list`
      with `labels`/`assignee`/`type` params; `braid_ready` likewise

Red state confirmed 2026-06-04: braid-core fails to compile (ListFilter
missing); CLI test fails on unknown `--label` flag.

## Phase 2 — Implementation

- [x] `braid-core/src/domain.rs`: add `ListFilter` + `matches`
- [x] `crates/braid/src/ops.rs`: thread filter through `list()` and
      `ready()` session methods
- [x] `crates/braid/src/main.rs`: clap flags on `List` and `Ready`
      (`--label` repeatable with `-l` short, `--assignee`, `--type`/`-t`)
- [x] `crates/braid/src/commands.rs`: `FilterOpts` (raw strings → domain
      `ListFilter` at the boundary); no output changes
- [x] `crates/braid/src/mcp.rs`: extend `braid_list`/`braid_ready` schemas
      and handlers with optional `labels`, `assignee`, `type` (shared
      `filter_params` schema + `FilterP` param struct)

All new tests green 2026-06-04 (cli, deps_cli, mcp_cli, braid-core unit).

## Phase 3 — Docs (same commit)

- [x] `crates/braid/src/agents-info.md`: update `list`/`ready` usage lines
- [x] `docs/mcp.md`: note new params on `braid_list`/`braid_ready`
      (Semantics worth knowing section)
- [x] README: skipped — flag additions, not conceptually significant

## Phase 4 — Wrap-up

- [x] `cargo xtask ci` green
- [x] Dogfooded on the repo skein (`list --type feature`,
      `ready --assignee claude`)
- [x] Update strand br-jd16my6w (comment + close) with `BRAID_AUTHOR=claude`
      — closed with reason "implemented in 3ec6faf"

## Resolved questions (Carlos, 2026-06-04)

1. MCP parity: **included in this strand.**
2. `--assignee`: **exact match only**; no unassigned sentinel (follow-up
   strand if ever needed).
3. `--type`: **arbitrary strings accepted**, matching `Other(...)` strands
   consistently with the schema's tolerance for custom types.
