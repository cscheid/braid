# braid agents-info skill installer (beads-AGENTS.md parity)

Strand: **br-4f2dk7ki** — "braid agents-info skill installer (beads agents
parity)" (P1 feature, child of epic **br-f2t3crsl**, braid 0.3.0 / q2
migration).

Source: `2026-06-08-braid-0.3.0-features-for-migration.md`, Feature 4.

## Overview

beads ships `br agents` to install an `AGENTS.md` workflow stub. braid has
`braid agents-info` (prints the version-matched guide) and the README
mentions installing "a one-paragraph skill that defers to it," but there's
no command to actually write that stub. q2 needs a concrete, idempotent
installer so a project's agents pick up braid usage without hardcoding
commands. The installed file's body is essentially: "run `braid
agents-info` for the authoritative, version-matched usage guide," plus the
one-paragraph workflow summary.

## What the code already gives us (confirmed 2026-06-08)

- `commands::agents_info()` (`commands.rs:856`) just
  `print!("{}", include_str!("agents-info.md"))` — the full guide.
- CLI arm `Cmd::AgentsInfo` (`main.rs:186,340`).
- The "one-paragraph workflow summary" already exists inside
  `agents-info.md` ("## The agent workflow" section) — we can lift/condense
  it rather than write new prose, keeping a single source of truth.

## Design decisions (to iterate on)

### Command surface

Add an `--install <dir>` flag (and optional `--force`) to `agents-info`:

```
braid agents-info                 # prints the full guide (unchanged)
braid agents-info --install DIR   # writes/updates the stub file in DIR
```

`DIR` is the directory; the installer writes a fixed filename inside it
(see path below). **Decided (Carlos, 2026-06-08): `--install <dir>` flag on
`agents-info`**, not a separate `braid agents` subcommand — keeps it one
concept and a smaller CLI surface.

### What file / where

The installer is **tool-host-agnostic in content**, but q2 will drop it
under `.claude/skills/braid/`. Decisions:

- Filename: **Decided — default `SKILL.md`** (Claude skill convention)
  written inside `DIR`. The installer only emits the file; q2 wires it into
  its skill system by pointing `--install` at `.claude/skills/braid/`.
  Coordinate the exact path with q2 if it diverges, but `SKILL.md` is the
  default and we don't need a filename override for 0.3.0 unless q2 asks.
- **Decided — create parent dirs if missing** (mkdir -p semantics) so
  `--install .claude/skills/braid` works in one shot.

### Idempotency (the core requirement)

Re-running must not duplicate. Use **delimiter markers** around a managed
block, preserving surrounding user content (the q2 `<!-- BEGIN/END WORKTREE
CONTEXT -->` pattern):

```
<!-- BEGIN BRAID (managed by `braid agents-info --install`) -->
... generated body ...
<!-- END BRAID -->
```

Install logic:

- File absent → create with just the managed block.
- File present with the markers → replace **only** the block between them,
  byte-for-byte preserving everything before/after.
- File present without markers → **append** the managed block (with a
  leading blank line), preserving existing content. **Decided (Carlos,
  2026-06-08): append**, not error — least destructive, matches "preserve
  surrounding user content."
- Malformed (BEGIN without END, or nested) → error clearly rather than
  guess. Test this.

### Managed block content

- A short heading + one paragraph: braid is the tracker; run `braid
  agents-info` for the authoritative, version-matched guide; the core
  workflow loop (`ready` → claim → `comment` → `close`; file discovered
  work with `create --deps`).
- Keep it intentionally thin so it never goes stale: the real guide is
  always `braid agents-info`. Avoid embedding the full command table.
- Source the paragraph from the existing `agents-info.md` workflow section
  (single source of truth) — e.g. a small `const` or a dedicated
  `include_str!` snippet so the installed text and the printed guide can't
  drift.

### Where the code lives

- **main.rs**: add `install: Option<PathBuf>` (+ maybe `force`) to the
  `AgentsInfo` command.
- **commands.rs**: `agents_info_install(dir, ...)` — pure-ish string
  assembly (managed-block splice) + a thin filesystem write. Factor the
  splice (`upsert_managed_block(existing: &str, body: &str) -> String`)
  into a **pure function** so it's unit-testable without touching disk.
- No braid-core change (this is CLI/filesystem, not schema/domain).

## Test plan (write first — TDD)

Pure splice fn (no I/O):

- [ ] Empty input → output is exactly the managed block (markers + body).
- [ ] Input with an existing managed block → block replaced; text before
      and after the markers preserved byte-for-byte.
- [ ] Input with trailing user content after END → preserved on re-install.
- [ ] Input without markers → block appended, prior content intact.
- [ ] Malformed markers (BEGIN, no END) → error.

Command / filesystem (tempdir):

- [ ] `--install <tmp>` → file exists at the expected path with the managed
      block; parent dir created if absent.
- [ ] Re-install → no duplication; block replaced; surrounding user content
      preserved.

## Work items

- [ ] Tests written and red (pure splice + tempdir install).
- [ ] `upsert_managed_block` pure function with marker handling.
- [ ] Managed-block body sourced from agents-info workflow section (no
      drift).
- [ ] `--install <dir>` (+ `--force`?) on `agents-info`; mkdir -p; write.
- [ ] Coordinate filename/path default with q2 (`SKILL.md` under
      `.claude/skills/braid/`).
- [ ] `cargo xtask ci` green (`docs_drift`: `agents-info` subcommand still
      listed; new flag documented).

## Docs to touch (same commit)

- `crates/braid/src/agents-info.md` — document `--install` and the managed-
  block behavior; replace the README's vague "how to install a one-
  paragraph skill" with the concrete command.
- README — update the install-skill mention to the real command.
