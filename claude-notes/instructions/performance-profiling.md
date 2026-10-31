# Performance profiling workflow

A playbook for diagnosing braid performance problems. Adapted from the q2
repo's `claude-notes/instructions/performance-profiling.md`, keeping the
native Rust workflow. Worked example:
`claude-notes/plans/2026/10/31/braid-list-performance.md`.

## Core rules

- **Measure on a real skein, then scale it.** One wall-time number can't tell
  O(n) from O(n²). Build a geometric series before reasoning about shape.
- **Find out what the size is measured in.** For braid the relevant unit is
  automerge *ops*, not strands. Each character of text is an op.
- **Numbers before fixes.** Write findings down before proposing a fix, then
  test candidate fixes instead of estimating them where that is cheap.
- **Don't guess at unsymbolicated frames.** See "System libraries" below.

## Protect real data

Skein contents and doc ids are secrets, and `braid` syncs by default.

- Experiment on a private copy of the cache: `cp -R ~/.cache/braid/store
  <scratch>/cache/` and set `BRAID_CACHE_DIR=<scratch>/cache`.
- **Anything that creates a skein from real data must not reach a server.**
  Set `BRAID_SYNC_URL=ws://127.0.0.1:1` and `BRAID_SYNC_TIMEOUT=0.05`, and pass
  `--sync-server ws://127.0.0.1:1` to `braid init`.
- Delete scratch copies when you finish. Don't quote strand content or doc ids
  in notes.

## Build

```bash
cargo build --profile=release-perf -p braid   # target/release-perf/braid
```

`release-perf` is `release` with symbols. Plain `--release` strips them, so a
profile of it is only raw addresses. The installed `braid` is stripped too.

## Record and read a profile

```bash
samply record -s -n --unstable-presymbolicate -o <scratch>/list.json.gz -- \
  target/release-perf/braid list > /dev/null
```

`-s` saves without starting a server, `-n` doesn't open a browser, and
`--unstable-presymbolicate` writes the `.syms.json` sidecar the scripts read.
Run it from the directory of the project you are measuring.

```bash
# self time: which leaf functions burn CPU
python3 claude-notes/instructions/analyze_profile.py <scratch>/list.json.gz --top 40
# inclusive, per thread: which operation the time is under
python3 claude-notes/instructions/analyze_inclusive.py <scratch>/list.json.gz --top 30
```

**Start with the inclusive view.** samod does its document work on tokio worker
threads, so the main thread shows only part of the cost, and a self-time table
scatters one operation across dozens of automerge and hexane functions.

## Isolate costs

- **Network:** compare a normal run with
  `BRAID_SYNC_URL=ws://127.0.0.1:1 BRAID_SYNC_TIMEOUT=0.05`. The offline run
  still includes the 50ms dial wait.
- **Automerge operations:** time them directly against a snapshot file with a
  small scratch crate; see `braid-list-perf-harness.rs` in the worked example.
- **Dependency changes:** try them in a copy of the workspace with
  `[patch.crates-io]`, leaving this repo's `Cargo.lock` alone.

## Scaling fixtures

Import slices and multiples of a real skein's `braid list --all --json` output
into fresh offline skeins (1/8x to 4x). Strip dependencies that point outside a
slice, and prefix ids in the multiples. Report ops as well as strands.

Check history shape too. An imported skein has almost no history, so if it
costs the same as the real one, history isn't the driver.

## System libraries

samply's sidecar doesn't symbolicate system libraries: `libsystem_*` frames stay
bare addresses. If a hotspot lands there, use macOS `sample`, which resolves them:

```bash
target/release-perf/braid <args> &
sample $! 4 -file <scratch>/s.txt
```

A q2 session guessed "malloc" from such a frame and was wrong: the real cause
was the global locale lock, taken by `snprintf`. A cheap experiment that rules
out a cause is useful evidence, not a dead end.

## Verifying a fix

Compare before and after on the same cache and scenario, confirm the change at
the behavior level (for example that a file is no longer rewritten), and run
`cargo xtask ci`.
