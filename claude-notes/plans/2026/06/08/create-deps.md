# braid create --deps <type>:<id>[,<type>:<id>...]

Strand: **br-9l24ee7o** — "braid create --deps <type>:<id>[,...]" (P1
feature, child of epic **br-f2t3crsl**, braid 0.3.0 / q2 migration).

Source: `2026-06-08-braid-0.3.0-features-for-migration.md`, Feature 2.

## Overview

Atomic create-and-link. beads supports `br create "..." --deps
discovered-from:<parent>`; braid's `create` has no such flag, forcing a
second `braid dep add`. q2's "file discovered work as you go" loop (exactly
the pattern in `agents-info.md`) uses the one-shot form heavily. Add a
repeatable / comma-separated `--deps <type>:<target-id>` to `braid create`
that, after the strand is created, runs each dependency through the
**existing `dep add` code path** so validation/normalization is identical.

## What the code already gives us (confirmed 2026-06-08)

- CLI `Create` command: `crates/braid/src/main.rs:33-54` (no deps flag).
- `ops::CreateOpts`: `crates/braid/src/ops.rs:141-149` (title, description,
  issue_type, priority, labels, slug, assignee).
- `Session::create`: `ops.rs:451`; `Session::dep_add` lives in the same
  impl (`commands::dep_add` → `ops`, see `ops.rs:756` `DepAdded`).
- `DependencyType::from(&str)` (`schema.rs:271`) tolerates unknown strings
  (→ `Other`), and dangling targets are legal ("never block") — so a
  missing target must **not** hard-fail; match `dep add`.
- `braid_create` MCP tool: `mcp.rs:207` / dispatch `mcp.rs:553-582`.

## Design decisions (to iterate on)

### Flag surface

`--deps <type>:<target-id>` — support **both** repeatable and
comma-separated, parsed uniformly:

```
braid create "x" --deps discovered-from:br-abc --deps blocks:br-def
braid create "x" --deps discovered-from:br-abc,blocks:br-def
```

clap: `#[arg(long = "deps", value_delimiter = ',')] deps: Vec<String>`,
each element then split once on the first `:` into `(type, target)`.

### Semantics — direction DECIDED

- The new strand **depends on** each target with the given type — i.e.
  `dep add <new-id> <target> --type <type>`, the same direction as
  `dep add`. So `--deps discovered-from:<parent>` records "this new strand
  was discovered from <parent>" (new → parent). **Decided (Carlos,
  2026-06-08): match beads exactly** — other projects may run similar
  migrations, so beads parity is a hard requirement here, not just a
  convenience.
- Build the dep edges **inside `create` itself**, in the same write that
  creates the strand (see Atomicity, below) — reusing `dep add`'s exact
  primitives (`resolve_issue`, `DependencyType::from`, `Dependency::key`)
  so validation/normalization is identical.
- A freshly minted id can't be an existing target, so no self-edge is
  possible at create time; and a brand-new strand's outgoing edges can't
  close a cycle (nothing pointed at it yet) — so no cycle warning is needed
  here, unlike `dep add`.
- `--json` output: unchanged created-strand JSON (the deps are visible in
  its `dependencies` map and via `dep list`). **Implemented as such.**

### Validation / errors

- Bad format (no colon, e.g. `--deps notacolon`) → clear error naming the
  offending token, **before** the session opens (parse `--deps` up front so
  nothing is created). Empty type or empty target → same error.
- Unknown dep type → allowed (→ `Other`), same as `dep add --type`; not
  rejected (matches the tolerant schema). Silent, like `dep add`.
- **Missing target → hard error, atomically (nothing created).** This is a
  **corrected decision (2026-06-08)**: the plan originally said "dangling
  targets are legal, no hard fail," but that conflated *import* (which
  tolerates dangling, by design) with *`dep add`*, which actually
  **rejects** a missing target as a typo guard (`dep_add_validates_targets_
  and_self_edges` asserts failure with "no issue"). beads' own `create
  --deps` warns-and-skips a missing target — but the user's beads-parity
  requirement was specifically about *direction* (new→target), not the
  failure mode. So `create --deps` follows braid's own strict, consistent
  `dep add` behavior: a missing target is rejected with the same "no issue"
  error. The schema's "dangling targets are legal" still holds for the
  paths that intentionally allow it (import, merges); the interactive add
  commands guard against accidental dangling.

### Atomicity

The edges are resolved and built **before** the strand is written, then the
strand is created with its `dependencies` map already populated — a single
`commit_one`. So a missing target on *any* dep fails the whole create with
nothing persisted (no orphan strand, no partial edges). This is stronger
than "create then N× `dep_add`" and avoids the partial-application window
entirely.

## Where the code changes

- **main.rs**: `deps: Vec<String>` on `Create` (`#[arg(long = "deps",
  value_delimiter = ',')]`); pass through. **Done.**
- **ops.rs**: `pub fn parse_dep_spec(&str) -> Result<(String, String)>`
  (shared by CLI + MCP); `deps: Vec<(String, String)>` on `CreateOpts`;
  `create` resolves + builds the `dependencies` map before constructing the
  Issue, then one `commit_one`. **Done.**
- **commands.rs**: `create` parses/validates `--deps` via `parse_dep_spec`
  up front (before opening the session), then passes the parsed pairs.
  **Done.**
- **mcp.rs**: `braid_create` schema gains an optional `deps` string array;
  the dispatch arm parses via `parse_dep_spec` and passes through. Same
  `<type>:<target>` element format as the CLI. `docs/mcp.md` updated.
  **Done.**

## Test plan (write first — TDD)

- [x] `create "x" --deps discovered-from:<id>` → resulting strand's
      `dep list` shows one outgoing edge to `<id>` typed `discovered-from`
      (and incoming on the parent). [`create_with_deps_links_atomically`]
- [x] Multiple `--deps` (repeated flag) and comma-separated → same edges.
      [`create_with_multiple_deps_repeated_and_comma_separated`]
- [x] Bad format `--deps notacolon` → error naming the token; **no strand
      created**. [`create_deps_bad_format_errors_and_creates_nothing`]
- [x] Missing target (`--deps blocks:br-ghost99`) → "no issue" error,
      **nothing created** (atomic). [`create_deps_missing_target_errors_
      atomically`] — supersedes the old "dangling succeeds" case (see the
      corrected Validation decision above).
- [x] Unknown type (`--deps weird:<id>`) → edge recorded with type `weird`.
      [`create_deps_unknown_type_is_recorded_verbatim`]
- [x] `parse_dep_spec` unit tests: split, trim, unknown-type, missing-colon,
      empty-parts. [`ops::tests`]
- [x] MCP: `braid_create` with `deps` attaches the edge; missing target
      fails atomically. [`create_with_deps_attaches_edges_atomically`]

## Work items

- [x] Tests written and red.
- [x] `--deps` flag on `Create` (main.rs), repeatable + comma-delimited.
- [x] `parse_dep_spec` + atomic edge-building in `ops::create` (reuses
      `resolve_issue` / `DependencyType::from` / `Dependency::key`).
- [x] Extend `braid_create` MCP schema with `deps` + update `docs/mcp.md`.
- [x] `agents-info.md`: show the flag + update the "file discovered work"
      example to the one-shot form.
- [x] `create --help` lists `--deps` with the `<type>:<id>` syntax (clap
      doc comment on the flag).
- [x] `cargo xtask ci` green.

## Docs to touch (same commit)

- `crates/braid/src/agents-info.md` — `create` flags row + the discovered-
  work example.
- `docs/mcp.md` — `braid_create` gains the `deps` property.
- README — optional one-liner.
