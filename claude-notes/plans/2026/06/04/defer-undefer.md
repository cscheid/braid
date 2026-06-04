# defer/undefer scheduling commands

Strand: **br-om28mvo7** — "defer/undefer scheduling commands" (P4 feature).

## Overview

The `deferred` status exists in the schema (`Status::Deferred`,
excluded from `ready` via `is_active()`) but nothing manages it: the only
way in or out is the generic `update --status`. beads has defer-until-date
semantics (`defer_until` field). The strand asks us to decide: dates (a
field addition) or just a bare status toggle?

## Recommendation: dates, evaluated at read time

Add an optional `defer_until` timestamp plus `braid defer` / `braid
undefer` commands. The "scheduler" is **purely computed at read time** —
no daemon, no write-on-read:

> a strand is *awake* iff `status.is_active()`, **or** status is
> `deferred` with a `defer_until` that has passed.

This matches braid's existing design stance ("all computed at read time
over the hydrated Skein", design decision D2: braid never manages a
daemon) and avoids CRDT churn from read commands mutating the document.
The bare toggle falls out as the degenerate case: `defer` with no
`--until` sleeps until an explicit `undefer`.

Why dates at all: "park this until the 0.2 release / until next Monday /
until the upstream fix ships" is the actual use of defer in agent
workflows — without a wake time, deferred strands are write-only memory
that nobody revisits. The cost is one optional field, and (see below) it
does **not** require a schema_version bump.

## Schema-compatibility analysis (why no version bump)

Verified against the current code:

- `hydrate` reads only known issue keys (`amdoc.rs:502-522`) — an old
  binary reading a document with `defer_until` ignores it, loses nothing.
- `reconcile_issue` touches only known keys at the issue level
  (`amdoc.rs:188-275`; the delete-unknown-keys loops are scoped to the
  `labels`/`dependencies`/`comments` sub-maps) — an old binary editing a
  strand **preserves** a `defer_until` it doesn't know about.
- Divergence in behavior of an old binary: it won't wake expired strands
  (it never did) and won't clear `defer_until` on status changes (stale
  value, harmless — same class as a stale `closed_at` would be). Both
  acceptable; `SCHEMA_VERSION` stays 1.

The **export contract** (`docs/schemas/strand.schema.json`,
`additionalProperties: false`) does need the new optional field —
downstream validators pinned to the old schema would otherwise reject
exports that carry it. Additive change, noted in the schema `$comment`.

## Semantics (decisions to iterate on)

1. **`braid defer <ids...> [--until <when>]`** — sets status `deferred`,
   sets/clears `defer_until`, bumps `updated_at`. Re-running on an
   already-deferred strand updates the date. Multiple ids, like
   `close`/`reopen`. Error on a `closed` strand ("reopen first").
2. **`braid undefer <ids...>`** — status → `open`, clears `defer_until`.
   Error if the strand is not `deferred` (explicit feedback for agents;
   unlike `reopen`, which doesn't check — worth discussing).
3. **Leaving `deferred` by any path clears `defer_until`**: `undefer`,
   `close`, `reopen`, and `update --status <anything else>`. Mirrors how
   `reopen` clears `closed_at`/`close_reason`. `update --status deferred`
   remains legal (bare defer, no date).
4. **Ready/blocked computation**: `domain::ready_issues` and
   `blocked_issues` gain a `now: &str` parameter (callers pass
   `now_rfc3339()`; tests pass fixed instants). Awake-but-still-deferred
   strands appear in `ready` with status `deferred` + `defer_until`
   visible, so agents can see *why* they surfaced. Comparison uses the
   existing `time::is_after`; an unparseable `defer_until` is conservative
   (never wakes). No `defer_until` → never wakes (explicit undefer only).
5. **Blocking is unchanged**: `is_terminal()` is still `closed`-only, so a
   deferred strand keeps blocking its dependents, expired or not. An
   awake deferred strand with active blockers lands in `blocked`, not
   `ready` — consistent with open strands.
6. **`--until` accepted forms** (parsed CLI-side into RFC 3339 UTC):
   - full RFC 3339 (`2026-07-01T09:00:00Z`)
   - bare date (`2026-07-01` → midnight UTC)
   - relative duration (`7d`, `36h`, `2w`)
7. **Display**: `list`/`show` render `deferred (wakes 2026-07-01)` for
   dated strands; JSON output carries `defer_until` naturally via serde.

## Out of scope

- Any daemon/scheduler or write-on-read auto-undefer.
- `list --awake`-style filters (br-jd16my6w covers richer filters).
- Remembering the pre-defer status (undefer always restores `open`).

## Work items

### Phase 1 — core schema + domain (tests first)

- [x] braid-core tests: `Issue.defer_until` serde shape (omitted when
      `None`, present as RFC 3339 string) in `schema.rs` /
      `roundtrip.rs` style
- [x] braid-core tests: amdoc hydrate/reconcile round-trips
      `defer_until`; reconcile-without-change generates no ops
- [x] braid-core tests (`tests/domain.rs`): with a fixed `now` —
      unexpired deferred excluded from ready; expired deferred included;
      dateless deferred never wakes; unparseable date never wakes;
      expired deferred with an active blocker appears in `blocked`
      (plus: deferral still blocks dependents; wake boundary inclusive;
      `is_awake` over all statuses)
- [x] Implement: `defer_until: Option<String>` on `Issue`; amdoc
      hydrate + reconcile; `ready_issues`/`blocked_issues` take `now`;
      `is_awake(issue, now)` helper in `domain.rs`

### Phase 2 — CLI commands (tests first)

- [x] CLI tests (new `defer_cli.rs`, 17 tests): `defer` with/without
      `--until`; all three `--until` forms (+ garbage rejected, strand
      untouched); multiple ids; id fragments; `defer` on closed errors;
      re-defer updates/clears the date; `undefer` restores open + clears
      date; `undefer` on non-deferred errors; `close`/`update --status`
      clear `defer_until`; `update --status deferred` still works and
      keeps an existing date; `ready` wakes expired and sleeps future;
      `show`/`list` render the wake time
- [x] Implement: `parse_until` in `braid-core::time` (unit-tested:
      RFC 3339 normalized to UTC, bare date, `Nh`/`Nd`/`Nw` durations);
      `Cmd::Defer`/`Cmd::Undefer` in `main.rs`; `commands::defer`/
      `commands::undefer` modeled on `close`/`reopen` (resolve+validate
      all before mutating); clearing logic in `update`/`close`/`reopen`;
      `wakes:` line in `show`, `[wakes <t>]` suffix in listings

### Phase 3 — contract, import/export (tests first)

- [x] `docs/schemas/strand.schema.json`: optional `defer_until`
      (`$defs/timestamp`), `$comment` notes the additive change
- [x] `tests/schema_contract.rs`: CLI-built skein gains a deferred strand
      (dated + dateless); malformed-record case for a bad `defer_until`
- [x] `tests/import_export.rs`: import a JSONL line carrying
      `defer_until` (beads + braid format, preserved as-is), export
      round-trips it
- [x] Implement: `RawIssue.defer_until` in `import.rs`; export needs no
      change beyond the schema field (serde-driven)

### Phase 4 — docs & wrap-up

- [x] `crates/braid/src/agents-info.md`: document defer/undefer and the
      read-time wake rule (command table rows + statuses convention note)
- [x] README command list — README enumerates no commands; nothing to do
- [x] Comment on strand br-om28mvo7 (BRAID_AUTHOR=claude); close on merge
