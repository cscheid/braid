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
- Apply deps **after** the strand is created, in order, reusing the
  `dep_add` path. Do it within the same `Session` so it's one sync at the
  end if practical; at minimum one logical operation from the user's view.
- Surface any resulting cycle warning exactly as `dep add` does
  (`DepAdded.cycles`), non-fatal.
- `--json` output: keep printing the created strand; consider including the
  added edges (or leave as-is and rely on `dep list`). Recommendation:
  unchanged strand JSON for 0.3.0; note the edges in human output
  (`created br-xyz (+2 deps)`).

### Validation / errors

- Bad format (no colon, e.g. `--deps notacolon`) → clear error naming the
  offending token, **before** the strand is created (parse `--deps` up
  front so we don't create a strand then fail).
- Empty type or empty target → error.
- Unknown dep type → allowed (→ `Other`), same as `dep add --type`; do not
  reject (keeps parity and avoids fighting the tolerant schema).
- Dangling target (id doesn't exist yet) → allowed, no hard fail.

Open question: should an unknown dep type *warn*? `dep add` currently
accepts silently. Recommendation: match `dep add` (silent) for parity;
revisit globally if we ever tighten dep-type validation.

### Atomicity note

`create` then N× `dep_add` is not a single CRDT transaction, but braid has
no atomic multi-op primitive and merges are conflict-free, so partial
application on a mid-flight crash is acceptable and self-heals. Parsing
`--deps` before creation prevents the only realistic "created but
mis-specified" case.

## Where the code changes

- **main.rs**: add `deps: Vec<String>` to `Create`; pass through.
- **ops.rs**: add `deps: Vec<(String, String)>` (parsed) to `CreateOpts`,
  *or* keep `CreateOpts` clean and have `commands::create` loop calling
  `session.dep_add` after `session.create`. Recommendation: parse in the
  command layer, loop `dep_add` — keeps `CreateOpts` a pure field bag and
  reuses the exact validated path.
- **commands.rs**: `create` printer parses/validates `--deps`, creates,
  then applies edges; report count.
- **mcp.rs**: extend `braid_create` schema with an optional `deps` array.
  **Decided (Carlos, 2026-06-08): include in 0.3.0** — the splice is small
  (one optional array property + a loop after create in the `braid_create`
  dispatch arm, `mcp.rs:553-582`). Same `<type>:<target>` element format as
  the CLI; update `docs/mcp.md`.

## Test plan (write first — TDD)

- [ ] `create "x" --deps discovered-from:<id>` → resulting strand's
      `dep list` shows one outgoing edge to `<id>` typed `discovered-from`.
- [ ] Multiple `--deps` (repeated flag) → multiple edges, correct types.
- [ ] Comma-separated form → same result as repeated flags.
- [ ] Bad format `--deps notacolon` → error naming the token; **no strand
      created** (assert list count unchanged).
- [ ] Dangling target (`--deps blocks:br-doesnotexist`) → succeeds; edge
      recorded; strand is still `ready`/listed (never blocked by dangling).
- [ ] Unknown type (`--deps weird:<id>`) → edge recorded with type `weird`
      (round-trips), no error.

## Work items

- [ ] Tests written and red.
- [ ] `--deps` flag on `Create` (main.rs), repeatable + comma-delimited.
- [ ] Up-front parse/validate `<type>:<target>`; create-then-link loop
      reusing `dep_add`.
- [ ] Human output reports added edges; cycle warnings preserved.
- [ ] Extend `braid_create` MCP schema with `deps` + update `docs/mcp.md`.
- [ ] `agents-info.md`: show the flag (and update the "file discovered
      work" example to the one-shot form).
- [ ] `create --help` lists `--deps` with the `<type>:<id>` syntax.
- [ ] `cargo xtask ci` green.

## Docs to touch (same commit)

- `crates/braid/src/agents-info.md` — `create` flags row + the discovered-
  work example.
- `docs/mcp.md` — `braid_create` gains the `deps` property.
- README — optional one-liner.
