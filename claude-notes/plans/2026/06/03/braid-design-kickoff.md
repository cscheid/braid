# braid: design kickoff — an automerge-centric issue tracker for LLM agents

**Date:** 2026-06-03
**Status:** design parameters set; no implementation yet
**Session:** first session of the project

## Overview

`braid` is a fresh, beads-inspired issue tracker whose source of truth is a
single **automerge document** per tracker, synced through an automerge sync
server (samod / automerge-repo wire protocol). The motivating problem: beads
requires checking `issues.jsonl` into a particular git branch, which fights
worktree- and multi-branch-based multi-agent workflows. With a CRDT as the
source of truth, parallel agents get replication and conflict resolution for
free, with no git involvement.

We studied two codebases (vendored under `external-sources/`):

- **beads_rust** — the domain model to borrow from: `Issue` struct, 12
  dependency edge types (4 affect ready-work), labels, comments,
  ready/blocked computation. Its 10k+ lines of SQLite cache, JSONL
  export/import, dirty tracking, witness metadata, and file locking are
  precisely the machinery automerge replaces.
- **samod** (v0.10) — Rust automerge-repo implementation. Provides: tokio
  `Repo` API, websocket dialer with backoff (compatible with
  `wss://sync.automerge.org`), filesystem storage (automerge-repo nodefs
  layout), `DocHandle::with_document` transactions, `repo.stop()` flush.
  Self-described WIP; we already maintain a fork (quarto-hub) and know its
  edges.

## Decisions made this session

### D1. Lineage: fresh tool, beads-inspired

New crate(s), new name, clean-room data model designed CRDT-first. No code
reuse from beads_rust and no CLI-compatibility obligation. Concepts borrowed
deliberately: issue shape, dependency types, ready/blocked semantics,
status/priority/issue-type vocabulary.

### D2. Process model: per-invocation sync, one server, no daemon — ever

Each CLI command:

1. loads whatever local cache exists (see D9),
2. applies the mutation,
3. dials the **one configured sync server**, exchanges sync messages
   (bounded by a timeout), and exits.

If the network is slow, the user's remedy is to run a **local automerge sync
server** (e.g. samod-based) and point braid at it; that local server is then
responsible for peering against remote servers. Braid itself never manages a
daemon. Offline operation = dial times out, work proceeds from local cache,
next successful sync converges.

### D3. v1 scope: core tracker

- `init`, `create`, `update`, `close`, `reopen`, `show`, `list`, `search`
- labels, comments
- dependencies + `ready` / `blocked` computation (client-side, at read time —
  no materialized cache at this scale)
- JSONL **import** (migrate an existing `.beads/issues.jsonl`) and **export**
  (backup / escape hatch; export-to-stdout doubles as the grep surface, so
  no FTS ambitions)
- `agents-info` (see D11)

Deferred: templates, agent_context inheritance, compaction, defer/scheduling,
MCP server, multi-repo routing, epics tooling beyond parent-child edges.

### D4. Secret discovery: layered — env > repo file > user config

The (sync server URL, document ID) pair is a **read/write bearer secret**.
Lookup order:

1. `BRAID_SYNC_URL` / `BRAID_DOC_ID` environment variables;
2. a gitignored `.braid.toml` found by walking up from cwd (like `.git`
   discovery);
3. `~/.config/braid/projects.toml` (user-level map, covers fresh worktrees
   with zero per-worktree setup).

**No repo-local state directory.** The only thing that may live in the repo
is the single gitignored secret file (and users who prefer env/direnv or the
user-level config need nothing in the repo at all).

### D5. Text merge semantics: automerge `Text` for prose

`description`, `design`, `acceptance_criteria`, `notes`, and comment bodies
are automerge `Text` — concurrent prose edits merge character-wise instead of
last-writer-wins. Consequence for the edit-as-JSON model: reads hydrate
`Text` → `String`; writes **diff old vs. new and apply splices** rather than
replacing the field. This is what `autosurgeon`'s `Text` reconcile does —
evaluating autosurgeon (vs. manual traversal) is a Phase 0 task.

Short scalar fields (`title`, `status`, `assignee`, timestamps, …) remain
plain LWW scalars.

### D6. ID scheme: bd-style, longer hash

`<prefix>-<base36>` ids, prefix configurable per tracker (default `br-`;
trackers migrated from beads may keep `bd-`). Generated from **random
entropy** at ~6–8 chars — long enough that concurrent creation across
replicas is collision-safe in practice without a uniqueness oracle. No
content-hash derivation, no dedup-by-content (those assumed a single
observer). Optional human slug segment (`br-fix-crlf-8cda`) as in beads.

### D7. Dependency edges: per-issue map

`issue.dependencies` is a **map** keyed by `"<depends_on_id>:<type>"` →
edge metadata (created_at, created_by, …). Issue JSON stays self-contained
(mirrors beads JSONL reading habits); concurrent insertion of the same
logical edge converges to one entry. Incoming edges are computed by scanning
(fine at this scale).

### D8. Name: braid / `br`

Interleaved strands — nods to both beads and CRDT merging. **Known
collision:** beads_rust's binary is also `br`. Acceptable since braid is
intended to replace beads in our workflows; revisit if dual-install matters.

### D9. Local cache: XDG dir keyed by `sha256(doc-id)`; never in the repo

Stateless invocations would re-download the doc's full change history every
command (automerge sync is incremental only relative to local state). So we
keep a cache — but the naïve layout leaks the bearer token, because the
standard automerge-repo storage model keys everything by
`[doc_id, chunk_type, chunk_id]` and the stock filesystem adapters (JS
nodefs and samod alike) splay the doc id straight into directory names.

Decision:

- Cache lives at `~/.cache/braid/docs/<hex(sha256(doc_id))>/` (XDG-aware),
  directory mode 700, files 600.
- Implemented as a thin samod `Storage` adapter that maps `key[0]` (the doc
  id) through SHA-256 before delegating to filesystem storage.
- Properties: shared across all clones/worktrees automatically; safe to
  `rm -rf` at any time (pure optimization); **zero mutable index state** —
  no mapping file, hence no locking story for concurrent agent invocations.
  (An indirection file mapping doc-id → opaque name was considered and
  rejected: same unlinkability, but reintroduces shared mutable state.)
- Someone holding the doc id can compute the hash and confirm cache
  presence — irrelevant, since the doc id already grants full server access.
- Caveat: cached automerge chunks contain issue text in plaintext.
  Mitigated by permissions now; **deferred hardening:** encrypt cache at
  rest with a key HKDF-derived from the doc id, making the cache unreadable
  without the secret it accelerates. Composes cleanly with the hash layout.
- `--no-cache` flag for fully stateless invocations (in-memory storage).

Ecosystem context (verified 2026-06-03): automerge.org's storage docs say
nothing about protecting document IDs; Ink & Switch describe the status quo
as "security through obscurity" (doc-id leak ⇒ world-writable) and are
building **Keyhive** (capability-based access control) + **Beelay** (E2EE
sync relay) as the long-term fix. Both experimental — watch, don't depend.

### D10. Timestamps: plain ISO-8601 LWW strings

`created_at` / `updated_at` / `closed_at` are writer-set RFC 3339 strings,
LWW under merge. A merge may carry one side's `updated_at`; accepted —
keeps us automerge-independent (easy future migration, trivial JSONL
export without rederiving times from doc history).

### D11. `braid agents-info`: self-documenting for LLM agents

A command that prints, to stdout, a markdown guide for LLM agents on how to
use braid. The guide ships inside the binary, so it is always
version-matched. Agent skills then reduce to a stable pointer:

```markdown
---
skill: use braid to track issues
---
braid is a program you can use to manage issues in a distributed manner.
Run `braid agents-info` to learn more.
```

`agents-info` includes instructions for installing that skill ("tying the
knot"). JSON output (`--json`) is available on every command; agents are
the primary audience.

### D12. Author identity: layered

`created_by` / comment `author` resolve as: `BRAID_AUTHOR` env var → an
`author` field in the secret/config file → `git config user.name` → OS
username. First hit wins.

### D14. Hydrate/reconcile: hand-written, no autosurgeon (resolved 2026-06-03)

autosurgeon (0.11, and git main as of today) pins `automerge ^0.8`, while
samod 0.10 requires `automerge ^0.9` — disjoint pre-1.0 ranges, so
autosurgeon's types cannot unify with the documents samod hands us short of
maintaining a fork. Decision: **manual hydrate/reconcile in `braid-core`**
against the automerge 0.9 API. Verified: workspace resolves to a single
`automerge 0.9.0`, and `Transactable::update_text` exists (built-in
diff-and-splice), giving D5's Text semantics without autosurgeon's
`similar`-based diffing. Our schema is one fixed shape, so the hand-written
code is small and we keep total control of merge-shape decisions
(reconcile-by-key for maps, `update_text` for prose, plain `put` for LWW
scalars).

### D13. `init` works offline

`br init` creates the document locally (in the cache via samod) and prints
the secret material; the doc is announced to the server on the first
successful sync. `br init --join <url+doc-id>` adopts an existing tracker.
Phase 2 must verify samod's create-then-dial flow announces correctly.

## Document schema (strawman v1)

One automerge document per tracker:

```jsonc
{
  "metadata": {
    "schema_version": 1,
    "name": "my-project",          // display name
    "id_prefix": "br",
    "created_at": "2026-06-03T...Z"
  },
  "issues": {
    "br-068k3x": {
      "id": "br-068k3x",            // duplicated for self-contained reads
      "title": "…",                  // LWW scalar
      "description": Text,           // automerge Text
      "design": Text,                // optional
      "acceptance_criteria": Text,   // optional
      "notes": Text,                 // optional
      "status": "open",              // open|in_progress|blocked|deferred|closed (LWW)
      "priority": 2,                 // 0..4 (LWW)
      "issue_type": "task",          // task|bug|feature|epic|chore|docs|question
      "assignee": "…",               // optional LWW
      "created_at": "…", "created_by": "…",
      "updated_at": "…",             // LWW, set by writer (see D10)
      "closed_at": "…", "close_reason": "…",   // optional
      "external_ref": "…",           // optional
      "labels": { "cargo": true, "deps": true },   // map-as-set (not array)
      "dependencies": {
        "br-t3ny:parent-child": {
          "depends_on_id": "br-t3ny",
          "type": "parent-child",
          "created_at": "…", "created_by": "…"
        }
      },
      "comments": {
        "c-9f3k2a": {                 // random id — never sequential ints
          "id": "c-9f3k2a",
          "author": "…",
          "created_at": "…",
          "text": Text
        }
      }
    }
  }
}
```

Schema notes:

- **Maps everywhere arrays would duplicate**: labels are a map-as-set;
  comments and dependencies are maps keyed by collision-free ids. (beads'
  integer comment ids are a CRDT hazard and are not carried over.)
- **Deletion** = removing the key from `issues` (automerge handles the
  tombstoning internally). beads' tombstone/ephemeral/pinned/template states
  are dropped from v1.
- **Dependency types** carried over from beads: `blocks`, `parent-child`,
  `conditional-blocks`, `waits-for`, `related`, `discovered-from`,
  `replies-to`, `duplicates`, `supersedes`, `caused-by`.
- **Blocking semantics** (refined during Phase 3, diverging deliberately
  from a literal reading of beads): `blocks` / `conditional-blocks` /
  `waits-for` block ready-work; **`parent-child` does not** — children are
  how an epic progresses, so an open parent must not stop work on them.
  Instead, open children gate the *parent's close* (close-protection).
  beads reaches a similar end state via its `:child-open` blocked-cache
  markers; braid makes it a first-class rule.
- **Ready** = status ∈ {open, in_progress} and no blocking edge whose
  target exists and is non-closed (dangling edges never block; blocking is
  one-step, so dependency cycles merely block their members). Computed in
  plain Rust over the hydrated doc.

## Crate layout

Cargo workspace:

- `braid-core` — schema types, hydrate/reconcile, ready/blocked + cycle
  detection, id-gen, identity resolution. **No I/O.**
- `braid` — CLI binary: config discovery, samod plumbing (cache storage
  adapter, dial/sync/exit), command surface, `agents-info` text.

## Risks / known tensions

- **Single-doc growth**: automerge keeps full history; a busy tracker grows
  without bound. Fine for O(10³) issues. Escape hatches if needed later:
  doc rotation (export → fresh doc), or index-doc + per-issue docs. Schema
  should not preclude this (hence self-contained issue objects).
- **Public sync server**: `wss://sync.automerge.org` stores data unencrypted
  and the doc ID is the only credential (security-by-obscurity, per the
  ecosystem's own framing). Fine for default/demo; real use should point at
  a self-hosted server. Document prominently; Keyhive/Beelay may eventually
  fix this upstream.
- **samod is pre-1.0**: API broke at 0.8 and 0.9/0.10; pin the version and
  budget for upgrades.
- ~~**autosurgeon compatibility**~~: resolved — incompatible (see D14);
  hydrate/reconcile is hand-written.

## Phased roadmap (TDD throughout — tests precede implementation in every phase)

### Phase 0 — scaffold + schema spike

- [x] Cargo workspace: `braid-core`, `braid` (CLI)
- [x] Pin samod 0.10 + automerge 0.9; evaluate autosurgeon vs. manual
      hydrate/reconcile → **manual** (version conflict; see D14)
- [x] Tests: hydrate/reconcile round-trip for the full Issue shape,
      including Text fields and map-shaped labels/deps/comments
      (13 tests in `tests/roundtrip.rs`; reconcile is idempotent and
      touches only what differs; Text spliced in place via `update_text`)
- [x] Tests: concurrent-merge semantics (two forks of a doc; edit same
      description → interleaved; same scalar → LWW; same logical dep edge
      added twice → single entry) — 10 tests in `tests/merge.rs`, including
      two pinning tests: **delete wins over concurrent edit**, and
      concurrent same-id creation converges with one object winning
- [x] id-gen module + collision-probability tests (`src/id.rs`: 8-char
      base36 suffix, optional slug, seeded deterministic collision test)

### Phase 1 — config, cache, local-only tracker

- [x] Layered secret discovery (env > `.braid.toml` walk-up > user config)
      + identity resolution (D12), with table-driven tests (16 tests,
      `braid/src/config.rs`)
- [x] Hashed-dirname cache `Storage` adapter (D9) + permission handling
      (8 tests incl. no-doc-id-on-disk walk; `braid/src/cache.rs`).
      `--no-cache` deferred to Phase 2: stateless invocations are
      meaningless without a network to sync from.
- [x] `br init` (create doc locally, write/print secret, `--join`),
      `br create`, `br show`, `br list` — all working offline against the
      cache (12 e2e tests, `braid/tests/cli.rs`; manual smoke verified
      cache paths contain only hashes)
- [x] Error story for missing/invalid secret (NoDocId guidance, invalid
      doc_id format, not-in-cache message pointing at Phase 2 sync)

### Phase 2 — sync

- [x] Per-invocation dial → sync → exit, with timeout + offline fallback
      (`braid/src/sync.rs`: ws/wss/tcp schemes, `BRAID_SYNC_TIMEOUT`
      default 5s; reads wait `we_have_their_changes`, writes wait
      `they_have_our_changes`; offline → stderr warning + cache fallback;
      explicit `braid sync` command treats offline as failure)
- [x] Integration tests against an in-process samod server — went one
      better than channel transport: a real samod Repo behind a TCP
      loopback listener, exercised by real binary invocations
      (7 tests, `braid/tests/sync.rs`)
- [x] Two-clone convergence test: separate HOME/cache per clone, both see
      both clones' issues through the server
- [x] `br init --join <doc-id>` (landed in Phase 1; fetch-on-first-use
      covered by the fresh-clone sync test)
- [x] Verify create-offline-then-announce flow (D13): init+create against
      a dead server, then sync to a live one; fresh clone fetches all
- [x] `BRAID_NO_CACHE=1` stateless mode (deferred from Phase 1) with a
      test proving statelessness both online and offline.
      Note: Phase 1 CLI tests pin a dead `tcp://127.0.0.1:1` server —
      pointing tests at the default public relay would leak test data.

### Phase 3 — domain features

- [x] `update`, `close`, `reopen`, labels, comments (empty string clears
      optionals; close protects against open children unless `--force`;
      multiple ids per close/reopen; 9 e2e tests)
- [x] `dep add/remove/list`, `ready`, `blocked` (+ cycle detection):
      braid-core `domain` module (17 unit tests) + CLI (9 e2e tests).
      Semantics refinement: parent-child is hierarchical, not blocking —
      see the Blocking semantics note in the schema section. Cycles are
      warned about at `dep add` but allowed (merges can create them
      anyway); `dep cycles` reports them.
- [x] `search` (case-insensitive substring over id/title/prose/labels/
      comments; `--json`)
- [x] `agents-info` (D11): version-matched markdown guide embedded in the
      binary (`src/agents-info.md`), including the pointer-skill snippet
      ("tying the knot")

### Phase 4 — migration + escape hatches

- [x] `braid import` from beads `issues.jsonl` (tolerant parser accepts
      beads arrays *and* braid maps; integer comment ids → fresh `c-` ids;
      `"completed"` → closed alias; beads-only fields dropped; upsert by
      id; parse-before-mutate atomicity; per-issue transactions — one big
      transaction was severely superlinear in automerge reads).
      Validated against the real 1121-issue example JSONL: 0.96s release
      import, all listings work, and `dep cycles` found two real cycles
      in the data. Dev profile now pins `opt-level=3` for automerge
      (10-100x difference).
- [x] `braid export` (JSONL to stdout, id-sorted; byte-exact
      export→import→export round trip is tested)
- [x] Docs: README with quick start, config layers, sync model, prominent
      doc-id-as-secret + public-relay security note, beads migration,
      agents-info pointer
- [ ] Deferred hardening (tracked, not scheduled): HKDF-encrypted cache

### Post-v1 observations

- One transient e2e failure ("not in the local cache" right after init)
  was seen exactly once, during a cargo relink mid-test-run after a
  Cargo.toml profile change; 3 subsequent full-suite runs were clean.
  If it recurs without build churn, investigate samod create/persist
  visibility between processes.
