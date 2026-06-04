# braid rotate: skein rotation for doc-growth and doc-id revocation

**Strand:** br-doc-growth-l1bracwo
**Date:** 2026-06-04
**Status:** complete (2026-06-04)

## Overview

One mechanism serves two needs:

- **Growth** (`braid rotate`, the default "compact" mode): automerge
  documents keep full history forever; rotation exports current state
  into a fresh document, shedding history. A **forwarding record** in the
  old document's metadata lets stale clones discover the successor.
- **Revocation** (`braid rotate --revoke`): the doc id is an irrevocable
  read/write bearer capability; if it leaks, rotation is the only
  recovery. **No forwarding pointer may be written** — the attacker reads
  the old document, and a pointer would hand them the new capability.
  The new secret is distributed out-of-band by humans.

Honest limits, documented loudly: rotation protects *future* reads and
writes only. The old document's full history remains readable forever to
anyone holding the old id (the relay keeps it); revocation does not
un-leak the past. And rotation is a cutover: writers who haven't synced
when it happens become "stragglers" whose changes land in the old
document — detected and surfaced, not silently lost.

## Design decisions

- **D-R1 — metadata fields**: `metadata.rotated_at` (timestamp; presence
  means "this skein was rotated") and `metadata.rotated_to` (successor
  doc id; **compact mode only**). Additive optional fields; document
  schema_version stays 1. `rotated_to` is itself a bearer capability —
  it is never printed by braid; `--adopt` moves it directly into
  `.braid.toml`.
- **D-R2 — rotation requires the server**: both the new document's push
  and (in compact mode) the old document's pointer write must be
  confirmed (`they_have_our_changes`) before braid touches local config.
  Offline rotation would fork from stale state; refuse.
- **D-R3 — failure-ordered cutover**: create new doc → push confirmed →
  write rotation marker to old doc → push confirmed → rewrite
  `.braid.toml` → report. An interruption leaves either an orphan new
  doc (harmless) or a marked old doc with the local config still old —
  which the rotation check then recovers via `--adopt`.
- **D-R4 — every command refuses a rotated skein**: `open_skein` checks
  `rotated_at` after pulling. Compact: "run `braid rotate --adopt`".
  Revoke: "ask a human for the new secret". This also stops braid from
  writing new strands into a dead document.
- **D-R5 — `--adopt`**: reads the old doc's pointer, rewrites
  `.braid.toml` in place (only when the doc id came from the repo file;
  env/user-config sources get instructions instead), then verifies the
  new skein loads. Never prints the new id.
- **D-R6 — straggler detection**: during `--adopt`, any strand in the old
  document with `updated_at > rotated_at` is a post-rotation write. They
  are written to `.braid-stragglers.jsonl` (braid export format) with
  instructions to review and `braid import` — automatic re-merging would
  silently clobber newer edits in the new skein, so a human (or agent)
  decides.
- **D-R7 — new doc id, new cache entry; the old cache is left alone**
  (it holds nothing the machine doesn't already have).
- **D-R8 — fresh `created_at`** in the new doc's metadata (it is a new
  document; provenance lives in the old one), same name/prefix/version.

## Work items

### Phase 1 — braid-core: rotation metadata
- [x] `SkeinMetadata.rotated_at` / `rotated_to` (Option<String>), hydrate +
      reconcile via `init_skein`, round-trip + idempotence tests
- [x] `hydrate_metadata` (metadata-only read for the cheap rotation check)

### Phase 2 — braid: open_skein refactor + rotation check
- [x] fold `pull()` into `open_skein` (every command pulls right after
      open today; rotation check must see the freshest metadata), keep a
      `open_skein_unchecked` for `--adopt`
- [x] rotated-skein error: compact vs revoke variants (e2e-tested wording)

### Phase 3 — braid rotate / rotate --adopt
- [x] `braid rotate [--revoke]` per D-R2/D-R3
- [x] `braid rotate --adopt` per D-R5/D-R6
- [x] e2e (in-process server): compact round trip (state preserved, new
      id in `.braid.toml`, fresh clone sees strands, stale clone refused
      then adopts); revoke (no pointer in old doc, distinct error);
      straggler detection writes the JSONL and names the strands;
      offline rotate refuses cleanly

### Phase 4 — docs
- [x] agents-info: rotate/adopt rows + rotated-skein guidance
- [x] README: rotation section (growth + revocation, pointer rule,
      honest limits)
- [x] design-kickoff doc: mark the doc-growth escape hatch resolved
