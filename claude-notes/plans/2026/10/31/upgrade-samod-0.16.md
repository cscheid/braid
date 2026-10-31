# Upgrade samod to 0.16 and automerge to 0.12

## Overview

Move braid from samod 0.10 / automerge 0.9 to samod 0.16 / automerge 0.12 with no
behavior changes (strand br-gicflc4t, under tracker br-pgfq9wo6), so performance
work compares against current upstream. Follows
`braid-list-performance.md`. Out of scope: the samod re-save bug report, which
waits on the `automerge.Text` design discussion, and fail-fast dialing.

## Phase 1: tests first

- [x] Add a test that the loom doesn't announce doc ids (the upgrade replaces this
      API, and nothing covered it); proved it fails with announcing enabled
- [x] Baseline: `cargo xtask ci` green on the old versions (331 passed, 1 ignored)

## Phase 2: upgrade

- [x] Bump samod and automerge
- [x] Port `WsDialer` to the `Dialer::Error` API (all failures stay transient)
- [x] Port the loom to `ShareConfig::read_write().with_announce(false)`
- [x] Re-verify the `load_range` prefix claim in `cache.rs` against samod-core 0.16
- [x] `cargo xtask ci` green (331 passed, 1 ignored)

## Phase 3: compatibility

- [x] 0.7.0 and new clients converge through a 0.7.0 loom and a new loom: all four
      combinations, with prose edits, comments and non-ASCII text
- [x] Each binary reads a cache written by the other (synthetic and q2)
- [x] JS automerge-repo 2.5.6 / automerge 3.2.6 reads and writes through the old and
      new looms, using a node script that writes scalars as the UI does. `braid ui`
      itself wasn't run in a browser
- [x] New client against `sync.automerge.org`: read-only runs on q2 cache copies

Not a compatibility problem, but worth knowing: new snapshots are about 1.1%
larger, and an old client rewrites the snapshot in the old form, so mixed versions
on one cache just flip it back and forth.

## Phase 4: re-measure

Same q2 scenarios, old and new alternating, each on its own cache copy. Best
median of two rounds:

| Scenario | 0.7.0 | Upgraded | Change |
|---|---:|---:|---:|
| `list`, online | 2.31s | 2.06s | -10% |
| `list --all --json`, online | 2.30s | 2.12s | -8% |
| `list --status closed`, online | 2.32s | 2.11s | -9% |
| `ready`, online | 2.29s | 2.07s | -9% |
| `list`, offline, 0.05s timeout | 2.02s | 1.81s | -10% |
| `list`, connection refused | 6.97s | 6.74s | -3% |
| `BRAID_NO_CACHE=1`, full fetch | 3.47s | 3.27s | -6% |

Output of `list` and `list --all --json` was byte-identical between versions on the
same cache state. The q2 skein is live and changed during this work, so compare
only runs against the same state.

Profile on the upgraded build, in on-CPU samples at about 1ms each: load 770,
`get_changes` 568, `save` 57, hydrate about 360. The re-save bug is unchanged and
is now about 30% of the run.

- [x] q2 baseline on the upgraded build
