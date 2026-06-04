# braid MCP server

**Strand:** br-s4ikmzr7 (P4; design analysis in its `design` field)
**Date:** 2026-06-04
**Status:** complete (2026-06-04, phases 0–3)

## Overview

An MCP server exposing a skein to agent harnesses, targeting the three
gaps the skill+CLI pattern cannot cover (in value order):

1. **Shell-less agents** — Claude Desktop / claude.ai web / IDE
   assistants / mobile have no CLI path; MCP is their only door.
2. **Capability isolation** — the server process holds the skein secret;
   the agent calls tools and *never possesses the doc id*. No
   `.braid.toml` to read, no `braid secret` to be talked into running.
   A harness can grant "issue tracking" without granting shell. This
   composes with the `DocId` work: you can't leak what you never see.
3. **Liveness** — a long-lived, *harness-managed* process (does not
   violate design D2: braid still never manages a daemon; the MCP host
   owns this lifecycle). It holds the samod repo open — no
   per-invocation dial — and can **push**: `DocHandle::changes()` →
   MCP resource-update notifications. "Tell me when my blocker closes"
   becomes possible.

For shell-capable agents the skill remains the preferred interface; the
MCP server is a second skin over the same operations, not a replacement.

## Architecture

```
braid-core            (unchanged)
braid/src/ops.rs      NEW: value-returning operations layer
braid/src/commands.rs becomes a thin printer over ops (CLI skin)
braid/src/mcp.rs      NEW: MCP skin over ops; `braid mcp` subcommand
```

### The ops extraction (the real work)

`commands.rs` functions print; MCP needs values. Introduce `ops` with a
session object and typed results:

```rust
pub struct Session { opened: OpenedSkein, /* author, … */ }

impl Session {
    pub async fn open(cwd: &Path) -> Result<Session>;        // dial once
    pub fn ready(&self) -> Result<Vec<Issue>>;
    pub fn list(&self, filter: …) -> Result<Vec<Issue>>;
    pub fn show(&self, query: &str) -> Result<Issue>;
    pub async fn create(&self, opts: CreateOpts) -> Result<Issue>;
    pub async fn update(&self, query: &str, opts: UpdateOpts) -> Result<Issue>;
    pub async fn close(&self, queries: &[String], reason: Option<String>, force: bool)
        -> Result<Vec<Issue>>;
    // … comment, reopen, dep_*, blocked, search, export
}
```

- **CLI**: opens a Session per invocation, prints, closes (today's
  behavior, refactored — zero user-visible change, full suite stays
  green before any MCP code lands).
- **MCP**: opens one Session at startup and holds it. samod's dialer
  already does long-lived reconnection with backoff (it was built for
  this); per-op hydrate from the live doc is cheap at our scale.

### Long-lived session semantics

- **Writes**: commit locally, then wait `they_have_our_changes` with a
  short timeout; the tool result carries `"synced": true|false` rather
  than blocking indefinitely — offline keeps working, results tell the
  truth.
- **Reads**: serve from the live doc (continuously synced); no
  per-call barrier.
- **Rotation**: re-check `rotated_at` (cheap `hydrate_metadata`) on
  every op; once a rotation arrives over sync, all tools fail with the
  rotation message ("ask a human" / adopt is deliberately *not* a tool).
- **Identity**: server-level (`BRAID_AUTHOR` / `--author` at launch).
  No per-call author parameter — agents don't get to impersonate.

## Tool surface (capability-scoped)

Included (names as the host sees them):

| tool | maps to |
|---|---|
| `braid_ready`, `braid_blocked`, `braid_list`, `braid_show`, `braid_search` | queries |
| `braid_create`, `braid_update`, `braid_close`, `braid_reopen`, `braid_comment` | mutations |
| `braid_dep_add`, `braid_dep_remove`, `braid_dep_list`, `braid_dep_cycles` | dependencies |
| `braid_export` | read-only JSONL escape hatch |

**The destructive-tool boundary** (Q3, resolved 2026-06-04 after an
ecosystem survey — GitHub MCP's toolsets/`--read-only`, the MCP tool-
annotations spec, official tracker servers): three layers, mirroring
what the popular servers converged on.

1. **Annotations on every tool** (2025-03-26 spec): `readOnlyHint` on
   queries; `close` is `destructiveHint: false` (reversible via
   `reopen`); mutations annotated honestly. Hosts build confirmation UX
   on these — but they are hints, not security.
2. **`braid mcp --read-only`**: force-disables all non-read-only tools
   (GitHub-style; e2e-tested, since GitHub shipped a transport where
   the flag silently failed — enforcement is real security surface).
3. **`delete` and `import`: excluded from the default toolset**, enabled
   only by an explicit launch flag (`--enable-destructive`). braid's
   delete has *no undo* (delete-wins CRDT, no tombstone) — strictly
   sharper than GitHub's `delete_file` (git history) or Linear's trash —
   so the *operator* opts in at launch; the agent never decides at
   runtime.

**Never tools, under any flag** — the isolation boundary proper:

- `secret` (defeats the entire point)
- `init`, `rotate`, `rotate --adopt` (operator decisions; an agent must
  never be able to rotate a skein or adopt a pointer)

Tool **outputs are strand records conforming to the published JSON
Schema** (docs/schemas/strand.schema.json) — the contract does double
duty. Tool **input schemas** give typed params (priority as 0–4 integer,
status enums-with-escape, etc.).

`braid mcp --read-only` serves only the query tools — cheap capability
tiering for untrusted agents.

## Resources & notifications (phase 3)

- Resources: `braid://ready` (the ready list), `braid://strand/{id}`,
  `braid://skein` (metadata: name, counts — never the doc id).
- `DocHandle::changes()` stream → `notifications/resources/updated` for
  subscribed resources (coarse v1: any remote change marks all
  subscribed resources updated; consumers re-read).
- This is the capability nothing else in braid offers; it's also the
  most protocol-fiddly part, hence its own phase after tools work.

## Protocol/SDK choice

Use **rmcp** (the official Rust MCP SDK): stdio transport, tool macros,
maintained against spec churn. It is pre-1.0 — same posture as samod:
pin it, keep the skin thin so churn stays absorbed in `mcp.rs`. The
fallback (hand-rolled ndjson JSON-RPC over stdio) remains viable if rmcp
disappoints; the plan isolates the dependency to one module either way.

Server runs **stdio only** in v1 (host-launched, host-authenticated). No
network listener: a TCP/SSE listener would reintroduce exactly the
bearer-capability problems we just spent two strands containing.

## Configuration

Host config launches `braid mcp` with cwd at the project root; the
normal layered discovery applies (env > `.braid.toml` walk-up > user
config + marker). An explicit `--project <dir>` flag covers hosts that
don't set cwd. Nothing new to learn; nothing secret in host config
beyond what `.braid.toml` already holds.

## Testing

- ops-extraction phase: the existing 155-test suite is the regression
  net; it must stay green with commands.rs as a thin printer.
- MCP e2e: spawn `braid mcp` as a child process, speak newline-delimited
  JSON-RPC over its stdio (initialize → tools/list → tools/call …),
  against the in-process sync server harness the sync/rotate tests
  already use. Assert: tool outputs validate against the JSON Schema;
  excluded tools are absent; `--read-only` hides mutations; doc id
  appears in **no** tool result, log line, or error (hygiene test, same
  spirit as tests/secret_hygiene.rs).
- Liveness e2e (phase 3): CLI creates a strand through the sync server →
  subscribed MCP client receives a resource-updated notification.

## Open questions (for plan review)

- **Q1 — session model confirmation**: long-lived single Session as
  described, or per-call open/close (slower, simpler, no liveness)?
  Plan assumes long-lived.
- **Q2 — rmcp vs hand-rolled**: plan assumes rmcp, pinned. Veto if the
  dependency posture feels wrong.
- ~~**Q3 — destructive-tool boundary**~~: resolved (cscheid,
  2026-06-04) — the three-layer pattern above: annotations + --read-only
  + delete/import behind --enable-destructive, default off.
- **Q4 — multi-skein**: one server = one skein (plan), or a
  `--project`-per-tool-call multiplexer? Multiplexing complicates the
  session model and the isolation story; plan says one skein per server
  process, run several servers for several projects.
- **Q5 — tool result shape**: full schema-conformant strand records
  (plan; reuses the contract, slightly verbose) vs trimmed summaries
  with a `braid_show` for detail?
- ~~**Q6 — does `braid_sync` exist as a tool**~~: resolved 2026-06-04 —
  no tool. Context: in the CLI, sync is a *moment* (short-lived process,
  dial-exchange-exit), so an explicit fail-loudly verb earns its place.
  The MCP session syncs continuously, so a sync tool degenerates into a
  busy-poll attractor. The agent's real question — "did my change reach
  the server?" — is answered by the `synced` flag on every mutation
  result; connection state and unconfirmed-changes count live in the
  read-only `braid://skein` resource (phase 3). If an explicit barrier
  ever proves necessary, a `readOnlyHint` `braid_status` tool can be
  added without design upheaval.

## Work items

### Phase 0 — ops extraction (no behavior change)
- [x] `ops::Session` + typed results; commands.rs becomes printers
      (ops.rs ~550 lines: Session with per-op rotation guard, Mutated<T>
      carrying PushOutcome, typed results for blocked/dep/delete/comment;
      OpenedSkein::push() non-consuming; check_rotation factored)
- [x] full suite green; no CLI output changes (155 tests, zero diffs —
      the e2e suite pins exact output text)

### Phase 1 — MCP server, tools
- [x] `braid mcp` subcommand (stdio, rmcp 1.7 — post-1.0, better than the
      plan assumed; manual ServerHandler impl for full control of
      annotations and gating; src/mcp.rs)
- [x] typed input schemas; outputs are schema-conformant strand records
      (CallToolResult::structured; braid_show output validated against
      docs/schemas/strand.schema.json in the e2e suite)
- [x] tool annotations on every tool; `--enable-destructive` gate for
      delete/import, default off (e2e: absent from tools/list AND refused
      at call time — the GitHub-bypass lesson, both directions tested)
- [x] rotation re-check per op (e2e: a *running* server starts refusing
      after another clone rotates the skein through the live relay);
      `sync` field on mutation results (confirmed|unconfirmed|offline)
- [x] `--read-only` (call-time enforced); `--project`; server-level
      identity via BRAID_AUTHOR (asserted in created_by)
- [x] e2e stdio harness: raw newline-delimited JSON-RPC client speaking
      initialize/tools-list/tools-call to the spawned binary
- [x] hygiene e2e: full protocol transcript asserted free of the doc id

### Phase 2 — docs
- [x] README section + docs/mcp.md (host setup snippets: Claude Code,
      Claude Desktop; capability tiers; semantics: structuredContent,
      sync field, rotation behavior, stdout discipline)
- [x] agents-info note (for shell agents: prefer the CLI; the MCP server
      deliberately serves a reduced surface)

### Phase 3 — resources & notifications
- [x] `braid://ready`, `braid://strand/{id}`, `braid://skein` resources
      (skein resource carries connection state + in_sync convergence —
      the Q6 status surface; PublicMetadata excludes rotation fields so
      `rotated_to` can never leak; hygiene e2e covers resource bodies)
- [x] changes() → resources/updated notifications (forwarding task owns a
      cloned DocHandle; bursts coalesced at 150ms); liveness e2e: a CLI
      write from another clone through a live relay pushes
      notifications/resources/updated to the subscribed running server
- [x] strand closed (all phases complete)

## Out of scope (recorded, not planned)

- Network transports (SSE/HTTP) — would reopen the capability-exposure
  problem; revisit only with real authentication.
- Prompts (`prompts/list`) — low value until a concrete host use case.
- Per-call impersonation/multi-user identity.
