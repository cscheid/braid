# `braid list` performance on a real skein (q2)

## Overview

How `braid list` performs on the q2 skein, and why. This is an investigation:
no fix has landed. Measured 2026-10-31 on braid 0.7.0 (samod-core 0.10.0,
automerge 0.9.0), on macOS, on a private copy of the cache.

Method: `claude-notes/instructions/performance-profiling.md`. Timing harness:
`braid-list-perf-harness.rs` in this directory.

## Checklist

- [x] Baseline in q2
- [x] samply profile, self-time and inclusive
- [x] Time automerge operations in isolation
- [x] Network and offline scenarios
- [x] Scaling series, 1/8x to 4x
- [x] Test candidate fixes: samod patch, summary hydrate, scalar text
- [ ] Decide which fixes to pursue
- [ ] Tests first, then implement what is chosen

## Findings

**The skein:** 2,672 strands (1,039 lines in the default listing), 2.5MB
snapshot, 7,063 changes, 5,725 actors, **4,026,638 ops**. Live text is 3.91M
characters (2.53M in issue fields, 1.38M in 1,927 comments), so about 97% of
ops are characters of current text. This isn't history bloat.

### Where 2.28s goes

| Phase | Time | Share | Source |
|---|---|---|---|
| Load snapshot (`load_incremental`) | 0.70s | 31% | harness |
| Redundant re-save after load (`get_changes` 0.64s + `save` 0.11s) | 0.75s | 33% | harness; patch removed 0.75-0.78s |
| `hydrate`, all strands and fields | 0.43s | 19% | harness |
| Network: dial, TLS, sync | ~0.30s | 13% | online minus offline |
| Everything else | ~0.10s | 4% | remainder |

Filters and output format make no difference, because everything is hydrated
before filtering: `list --all --json` 2.24s, `list --status closed` 2.26s,
`ready` 2.22s (medians). All commands that open a skein pay the load and re-save.

### The redundant re-save

`OnDiskState::last_saved_heads` starts as `None` and isn't set on load. So the
first `step()` after loading runs `get_changes(&[])`, regenerating the entire
history. That count exceeds 10, so it compacts: `doc.save()`, then a write to
the same content-addressed path. Observed: `braid list` updates the snapshot's
mtime and leaves its md5 unchanged.

Source: `samod-core-0.10.0/src/actors/document/on_disk_state.rs`
(`save_new_changes`) and `document_actor.rs`. Tested patch:

```diff
--- a/src/actors/document/document_actor.rs
                     self.on_disk_state
                         .add_keys(snapshots.into_keys().chain(incrementals.into_keys()));
+                    self.on_disk_state
+                        .mark_loaded_heads(self.doc_state.document().get_heads());
--- a/src/actors/document/on_disk_state.rs
+    pub(super) fn mark_loaded_heads(&mut self, heads: Vec<automerge::ChangeHash>) {
+        if !heads.is_empty() { self.last_saved_heads = Some(heads); }
+    }
```

### Scaling: linear in ops

From fresh offline skeins, so no network cost:

| Slice | Strands | Ops | `list` |
|---|---:|---:|---:|
| 1/8x | 334 | 174,695 | 0.19s |
| 1/4x | 668 | 570,924 | 0.36s |
| 1/2x | 1,336 | 1,670,969 | 0.85s |
| 1x | 2,672 | 3,972,427 | 1.84s |
| 2x | 5,344 | 7,944,848 | 3.56s |
| 4x | 10,688 | 15,889,690 | 7.21s |

About 0.45s per million ops. The slices show ops is the right unit: the strands
don't carry equal text, so ops double unevenly while cost follows them.

The imported 1x has 2,673 changes from 2 actors, against 7,063 from 5,725 in the
real skein, yet its load is only 8% cheaper (641ms vs 701ms). History shape is
at most a minor factor. Heads verification isn't a factor either:
`load_unverified_heads` takes 703ms vs 708ms.

### Network and offline

| Scenario | Median |
|---|---:|
| Online | 2.28s |
| Offline, `BRAID_SYNC_TIMEOUT=0.05` | 1.98s |
| `BRAID_NO_CACHE=1` (full fetch from the server) | 3.33s |
| Offline, connection refused, default timeout | **6.93s** |

The last row is a separate problem: a refused connection is retried until the
whole 5s timeout expires, even though the failure is immediate. Every command
pays this while the server is down.

## Candidate fixes

| Fix | Measured | `list` afterwards |
|---|---|---|
| A. Seed `last_saved_heads` on load | online 2.3s to 1.5s, offline 2.06s to 1.27s | **1.53s measured** |
| B. Summary-only hydrate for plain `list` | hydrate 436ms to 85ms | ~1.2s with A (estimate) |
| C. Store text as scalar strings | ops 4.03M to 56K, load 701ms to 41ms | ~0.65s with A (estimate) |
| D. Fail fast on connection refused | not tested | removes up to 5s when offline |

**A** was verified: the patched binary doesn't touch the snapshot, a 13-strand
offline skein persisted and read back correctly through the stock binary, and
the workspace tests pass on the patched copy (xtask excluded). It's a small
change to a dependency, so it needs an upstream PR, with `[patch.crates-io]` or
a git dependency until that ships.

**B** only helps when no field beyond the summary is needed. `--json`, `ready`
(dependencies) and `--label` (labels) still need more. The 85ms covers id,
title, status, priority, type and assignee.

**C** is the only option that changes the slope, but it's a real tradeoff:
concurrent edits to a description or comment would become last-writer-wins
instead of merging by character, and existing skeins would need migrating. The
measurement used a fresh single-writer document and walked every field (191ms),
so treat it as an upper bound on the gain.

**Not recommended yet: a hydrated-view cache keyed on heads.** It could skip load
and hydrate, but design decision D2 syncs on every command, and syncing needs the
loaded document. It only pays off if reads may skip or defer the sync.

## Is it samod or braid?

Checked against `external-sources/samod` (0.16.0, automerge 0.12.0).

**The re-save is samod's.** A minimal program with no braid code (create a
50-change document, then open it read-only three times) writes one snapshot per
open on both samod 0.10.0 and 0.16.0. `save_new_changes` is unchanged in 0.16;
`on_disk_state.rs` was last touched in August 2025. `RepoBuilder` offers storage,
runtime, peer id, share policy, concurrency and observer, and none of these
affects it, so braid can't configure it away. Upstream's
`many_changes_compact_but_do_not_delete_existing_snapshot` says a lookup "will
trigger a compaction", so load-time compaction is known behavior, at least with
many leftover incrementals. The rewrite of an already-compact snapshot looks like
a side effect of that rather than a goal, though that is an inference.

**Upgrading doesn't fix it.** Same q2 snapshot, automerge 0.9.0 vs 0.12.0:

| Operation | 0.9.0 | 0.12.0 |
|---|---:|---:|
| `load_incremental` | 690ms | 699ms |
| `get_changes(&[])` | 630ms | 555ms |
| `save` | 107ms | 55ms |
| Walk every field | 400ms | 325ms |

**The rest comes from braid's design, and those choices were deliberate:**

- D5 makes prose automerge `Text`. That accounts for about 97% of ops, and so for
  most of the load and text-hydrate cost.
- D2 and the no-daemon design load the whole document in every process. samod
  appears aimed at long-lived processes, where a one-time load and re-save cost
  matters little. The design doc's latency remedy (a local relay) addresses
  network time, not this.
- `hydrate` reads every field before filtering. A plain walk of the document with
  the same automerge calls takes 400ms against hydrate's 430ms, so hydrate isn't
  wasteful in how it reads, only in how much it reads.

No misconfiguration turned up. The fail-fast gap on refused connections is
braid's: `sync.rs` sets `max_retries: None` and relies on the outer timeout.

The patch ported to a scratch copy of samod 0.16 passes `cargo test -p samod-core`
(115 passed, 0 failed), including both compaction tests.

## Is character-level merging actually used?

Every write goes through braid. On the CLI and MCP that means
`reconcile_issue`; in `braid ui` it means `StrandDetail.tsx` and
`NewStrandDialog.tsx`. Writes that create or edit `Text`:

| Operation | `Text` effect |
|---|---|
| `create`, `import`, UI new strand | Creates `Text` for prose fields |
| `comment`, UI add comment | Creates `Text` for the body; no command edits a comment |
| `update --description/--design/--acceptance-criteria/--notes` | `update_text`, diffing old against new |
| UI edit of description, criteria, notes | Assignment (`r.description = v`) |
| status, labels, dependencies, title, assignee | Scalars and maps; no `Text` |

History of the q2 skein, decoded change by change. A document with deliberately
concurrent edits reported them correctly, so the analyzer can see them.

| Measure | Count |
|---|---:|
| Changes | 7,068 |
| Create `Text` without editing any | 4,575 |
| Scalars and maps only | 2,404 |
| **Edit an existing `Text`** | **89 (1.3%)** |
| `Text` objects ever edited after creation | 76 of 4,701 |
| Comment bodies ever edited | 0 of 1,956 |
| Characters written at creation | 3,904,539 |
| Characters inserted / deleted by edits | 54,525 / 14,665 |
| Concurrent change pairs in the whole history | 6,284 |
| ... touching the same strand | **0** |
| **Concurrent edits to the same `Text`** | **0** |

Limits: this is one skein, and most changes have no timestamp (6,868 of 7,068 read
as 1970), so the data can't show change over time. A zero also doesn't show what
would happen with heavier concurrent use.

Seven values that should be scalars were written as `Text` in q2 (`status` 2,
`updated_at` 3, `assignee` 1, `closed_at` 1). Plain JS string assignment produces
exactly this, which fits the UI's move to `RawString`, but the source of these
writes wasn't confirmed.

The UI's assignment doesn't merge. Concurrent edits of `"hello world"` in the JS
library give `"hello there world"` by assignment (one side lost) and
`"hello there world!!"` by `updateText`. So only CLI and MCP updates get character
merging today.

Constraints for a schema change, from reading the code (not tested):

- Ops already in history stay there, so converting fields in place won't shrink an
  existing skein. Getting the gain needs a new document, such as through `rotate`.
- `reconcile_text` replaces any non-`Text` value with new `Text`, even when the
  content is unchanged. Clients at 0.7.0 would turn scalar prose back into `Text` on
  every strand they update, including all of that strand's comments.

## Recommendation

Update, same day: samod 0.16 / automerge 0.12 is done separately (see
`upgrade-samod-0.16.md`) and takes `list` to 2.06s. Next, consider designs for moving
away from `automerge.Text` and what they would gain. The samod bug report waits until
after that, and the fix order above should be revisited with those results.
