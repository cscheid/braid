# braid import: recognize & skip beads tombstones

Strand: **br-arhr3j81** — "braid import: recognize & skip beads tombstones"
(P1 feature, child of epic **br-f2t3crsl**, braid 0.3.0 / q2 migration).

Source: `2026-06-08-braid-0.3.0-features-for-migration.md`, Feature 3.

## Overview

beads JSONL can carry soft-deleted records: `"status": "tombstone"` plus
`delete_reason` / `deleted_at` / `deleted_by`. braid's `convert_status`
(`crates/braid/src/import.rs:114`) only maps `completed`→`closed`, so
`tombstone` falls through to `Status::Other("tombstone")`
(`schema.rs:134,168`) — neither active nor terminal, so it shows up as
**noise in `braid list`**. q2's real JSONL has exactly 2 such records
(`bd-1xf5`, `bd-298oe`); both currently appear in `braid list`. Fix:
detect tombstones during import and **skip** them (create no strand), and
report the skipped count.

## What the code already gives us (confirmed 2026-06-08)

- `RawIssue` deserialization (`import.rs:28-68`) currently ignores all
  fields except a known set; unknown fields (incl. `deleted_at`,
  `delete_reason`, `deleted_by`) are dropped via serde's default behavior.
  We must **add** these as optional captured fields to detect them.
- `convert(raw, now) -> Issue` (`import.rs:137`) builds the strand;
  `parse_jsonl` (`import.rs:228`) is the pure parse loop, returns
  `Vec<Issue>`, no I/O — the right place to filter.
- The count message is printed in `commands::import` (`commands.rs:820`)
  from `Imported { imported: usize }` (`ops.rs:102,795`).

## Design decisions (to iterate on)

### Detection (be conservative)

Treat a record as a beads tombstone — and **skip** it — iff:

- `status == "tombstone"`, **OR**
- any of `deleted_at`, `delete_reason`, `deleted_by` is present and
  non-empty.

The doc's edge case: a record with `deleted_at` present but
`status:"closed"`. **Decided (Carlos, 2026-06-08): treat as tombstone /
skip**, because beads writes both on a delete. Document this so it's a
deliberate choice, not an accident. We are conservative in that we only key off the
beads-specific deletion fields — a plain `closed` strand with none of them
imports normally.

### Reporting

Change the count from a plain `Vec<Issue>` to a richer parse result so the
command can print:

```
imported 1143 strands (skipped 2 tombstones) from <file>
```

**Decided (Carlos, 2026-06-08): omit the suffix when M==0.** When no
tombstones were skipped, print the existing message verbatim (`imported N
strands from <file>`) so braid→braid round-trips stay visually unchanged
and existing tests don't churn; append `(skipped M tombstones)` only when
M>0.

### Plumbing the count

`parse_jsonl` is pure and returns `Vec<Issue>`. Options:

1. Return a struct `ParseOutcome { issues: Vec<Issue>, skipped: usize }`
   from `parse_jsonl` and thread `skipped` to the printer.
2. Keep `Imported { imported }` and add `skipped` there, set by the command
   from the parse outcome.

Recommendation: option 1 for the parse count (it's a parse-time fact),
surfaced through the printer; `Session::import` still just upserts the
issues it's given. Keeps the skip decision in the pure, well-tested parse
layer.

### Already-imported / resurrection edge case

If a previously-live id is now a tombstone in a re-imported file: **skip**
it (do not upsert), leaving any prior strand untouched. Deletion is a
separate explicit op (`braid delete`). Document this; it means import never
deletes — it only adds/updates non-tombstone records. (Matches braid's
stance that delete is explicit and wins over concurrent edits.)

## Round-trip safety

braid's own `export` never emits tombstones (no such status in the export
contract), so braid→braid round-trips are unaffected. Existing import
upsert/round-trip tests must still pass.

## Test plan (write first — TDD)

Unit tests in `import.rs` (pure `parse_jsonl` / detection helper):

- [x] JSONL with one normal + one `status:"tombstone"` record → result has
      1 issue, `skipped == 1`.
- [x] Record with `deleted_at` present but `status:"closed"` → skipped
      (treated as tombstone), per documented choice.
- [x] Record with `delete_reason` / `deleted_by` only → skipped.
- [x] Clean braid export (no deletion fields) → `skipped == 0`, all issues
      imported (round-trip unaffected).
- [x] A normal `closed` strand with **none** of the deletion fields →
      imported, not skipped (conservative detection).

Command-level:

- [x] `braid import` of a 2-record file prints the skip count message.
- [x] Existing import round-trip / upsert tests still green.

Acceptance (epic-level, validated at release): importing q2's real
`.beads/issues.jsonl` (1145 issues) imports 1143 strands, skips exactly 2
tombstones (`bd-1xf5`, `bd-298oe`), 1051 deps intact, clean `braid list`.

## Work items

- [x] Tests written and red.
- [x] Capture `deleted_at` / `delete_reason` / `deleted_by` on `RawIssue`.
- [x] `is_tombstone(&RawIssue)` detection helper (conservative).
- [x] `parse_jsonl` skips tombstones, returns issues + skipped count.
- [x] `commands::import` prints `(skipped M tombstones)` per chosen rule.
- [x] Document the resurrection / `closed`+`deleted_at` decisions (code
      comment + agents-info note if user-facing).
- [x] `cargo xtask ci` green.

## Docs to touch (same commit)

- `crates/braid/src/agents-info.md` — note that `import` skips beads
  tombstones (one line near the `import` row).
- `docs/schemas/` — no braid schema change (tombstone is a beads input
  concept, never an exported braid status); confirm no contract test needs
  touching.
