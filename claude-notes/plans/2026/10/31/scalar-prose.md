# Store prose fields as scalar strings

## Overview

Replace automerge `Text` with scalar strings for `description`, `design`,
`acceptance_criteria`, `notes` and comment `text` (strand br-4gi4yphu). Supersedes
design decision D5. Evidence: `braid-list-performance.md`: 97% of q2's ops are text
characters, and its 7,068 changes contain no concurrent edits to the same `Text`.

## Design

- **Writes:** prose is written as a scalar. An edit to legacy `Text` replaces it with
  a scalar. Matching legacy `Text` is left alone and generates no ops.
- **Reads:** scalars and legacy `Text` are both read, so no conversion is forced.
- **`schema_version` stays 1:** hydrate requires an exact match, so a bump would stop
  0.7.0 clients reading at all. 0.7.0 already reads scalar prose.
- **Migration is `braid rotate`:** it rebuilds through `reconcile_issue`. Existing
  skeins keep their history and their speed until rotated.
- **Merge semantics:** concurrent edits to one prose field are now last-writer-wins,
  with one side's whole value surviving. Concurrent new comments still both survive.

## Checklist

- [x] Tests: scalar storage, op count independent of length, legacy `Text` unchanged
      and edited, concurrent edits don't interleave
- [x] `reconcile_prose`
- [x] UI writes scalars for prose and comments
- [x] Docs: agents-info, syncing, rotation, schemas
- [x] `cargo xtask ci` (335 passed, 1 ignored)
- [x] Measure on a copy of q2
- [x] Run `braid ui` in a browser (Carlos, on this repo's skein, on a legacy strand):
      description edit and new comment stored as `Str`, cleared notes removed, old
      comment still `Text`, no conflicts, one head, hydrates on the new build and 0.7.0
- [ ] Decide when to release and rotate the real q2 skein

## Measurements

A copy of q2 (2,673 strands), rotated with the real `braid rotate` through a loopback
loom, in a throwaway `HOME` and cache. Same binary for both columns (samod 0.16).
`list --all --json` was byte-identical before and after rotation.

| | `Text` skein | Scalar skein |
|---|---:|---:|
| Ops | 4.04M | 61,933 |
| Changes | 7,068 | 2,674 |
| On disk | 2.54MB | 2.03MB |

Best median of two alternating rounds:

| Command | `Text` | Scalar | Change |
|---|---:|---:|---:|
| `list`, offline | 1.782s | 0.370s | -79% |
| `list`, loopback loom | 1.811s | 0.369s | -80% |
| `list --all --json`, offline | 1.824s | 0.372s | -80% |
| `ready`, offline | 1.842s | 0.371s | -80% |
| `search`, offline | 1.815s | 0.368s | -80% |
| `BRAID_NO_CACHE=1`, full fetch, loopback | 1.886s | 0.420s | -78% |
| `create`, offline | 1.854s | 0.378s | -80% |
| `comment`, offline | 1.840s | 0.385s | -79% |
| `update --description`, offline | 1.825s | 0.391s | -79% |

Not measured: the public server, because that would mean uploading a copy of q2.

Scaling on fresh scalar skeins, by multiplying q2's strands:

| Strands | `list` | Import |
|---:|---:|---:|
| 2,673 | 0.385s | 1.0s |
| 5,346 | 0.659s | 1.8s |
| 10,692 | 1.221s | 3.6s |
| 21,384 | 2.376s | 7.5s |

That is about 0.11s per 1,000 strands. The `Text` schema took 7.2s at 10,688 strands.

Profile of `list` on the scalar skein, in on-CPU samples at about 1ms each: hydrate
165, re-save 72 (`save` 51, `get_changes` 21), dial 43, load 37.

## Mixed versions

A 0.7.0 client on a scalar skein reads identically, and the new client reads what it
writes. But it converts prose to `Text` on the strands it updates: three updates to
one strand added about 9,800 ops (61,933 to 71,741). This is bounded by the prose on
those strands and loses no data, but it erodes the gain until the next rotation.
