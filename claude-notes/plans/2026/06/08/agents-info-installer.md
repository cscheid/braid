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

## Correction (post-0.3.0, br-6lgfaus0) — emit YAML frontmatter

The q2 agent found the installed `SKILL.md` had no frontmatter. Per the
Claude Code docs a body-only skill *does* load (the description falls back
to the first paragraph), so "not discoverable" was overstated — but without
an explicit `name`/`description` auto-invocation is unreliable, and every
other skill in this repo uses frontmatter. **Decided: emit frontmatter.**

Crucially, YAML frontmatter must be the file's **first bytes**, which the
original "append a managed block, preserve surrounding content" design
can't satisfy (a leading `<!-- BEGIN -->` comment breaks frontmatter
parsing). So the model changed:

- The braid-managed **head** = frontmatter (`name` from the install dir's
  base name; a `description` naming the triggers) + the body between
  markers, always at offset 0.
- Reinstall refreshes that whole head and **preserves content after the END
  marker** (a user can append below); it no longer preserves content
  *before* the block (frontmatter must lead).
- A non-empty `SKILL.md` with no braid markers → **refuse** (don't clobber a
  file braid didn't write; can't safely merge under the frontmatter rule).
  This replaces the old "append to any existing file" behavior.

Shipped in 0.3.1.

## Test plan (write first — TDD)

Pure splice fn (`commands::tests`, no I/O):

- [x] Empty input → output is exactly the managed block + trailing newline.
      [`empty_input_yields_just_the_block`]
- [x] Existing block → replaced; text before and after preserved
      byte-for-byte, exactly one block.
      [`existing_block_is_replaced_preserving_surroundings`]
- [x] Re-install is idempotent (running twice = running once).
      [`reinstall_is_idempotent`]
- [x] No markers → block appended with one blank-line separator, prior
      content intact. [`no_markers_appends_and_keeps_user_content`]
- [x] Malformed markers (BEGIN-only, END-only, reversed) → error.
      [`malformed_markers_error`]

Command / filesystem (`tests/agents_info.rs`, tempdir):

- [x] `agents-info` (no flag) still prints the guide.
      [`agents_info_prints_the_guide`]
- [x] `--install <dir>` → `SKILL.md` exists with the managed block; parent
      dirs created. [`install_writes_skill_with_managed_block`]
- [x] Re-install → no duplication; user content before and after the block
      preserved. [`reinstall_does_not_duplicate_and_preserves_user_content`]

## Work items

- [x] Tests written and red (pure splice + tempdir install).
- [x] `upsert_managed_block` pure function with marker handling (BEGIN/END
      delimiters; malformed → error).
- [x] Managed-block body defers to `braid agents-info` rather than copying
      the command table — cannot drift. (Decided against programmatically
      slicing agents-info.md: fragile; a thin pointer is the sound choice.)
- [x] `--install <dir>` on `agents-info`; `create_dir_all`; write `SKILL.md`.
      (No `--force` — replacing only the managed block is already safe.)
- [x] Default filename `SKILL.md`; q2 points `--install` at
      `.claude/skills/braid/` (installer is host-agnostic).
- [x] `cargo xtask ci` green.

## Docs to touch (same commit)

- `crates/braid/src/agents-info.md` — document `--install` and the managed-
  block behavior; replace the README's vague "how to install a one-
  paragraph skill" with the concrete command.
- README — update the install-skill mention to the real command.
