# braid MCP server

**Strand:** br-s4ikmzr7 (P4; design analysis in its `design` field)
**Date:** 2026-06-04
**Status:** plan for review — no implementation yet

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

**Deliberately excluded** — the isolation boundary:

- `secret` (defeats the entire point)
- `init`, `rotate`, `rotate --adopt` (human/operator decisions; an agent
  must never be able to rotate a skein or adopt a pointer)
- `delete` (sharpest mutation, delete-wins semantics; `close` covers
  agent workflows) — *open question Q3 below*
- `import` (bulk overwrite; operator action) — *also Q3*

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
- **Q3 — destructive-tool boundary**: plan excludes `delete` and
  `import` from the tool surface. Include them (with `force`-style
  params), or keep the conservative boundary?
- **Q4 — multi-skein**: one server = one skein (plan), or a
  `--project`-per-tool-call multiplexer? Multiplexing complicates the
  session model and the isolation story; plan says one skein per server
  process, run several servers for several projects.
- **Q5 — tool result shape**: full schema-conformant strand records
  (plan; reuses the contract, slightly verbose) vs trimmed summaries
  with a `braid_show` for detail?
- **Q6 — does `braid_sync` exist as a tool**? With continuous sync it's
  nearly redundant; plan omits it and reports `synced` per mutation.

## Work items

### Phase 0 — ops extraction (no behavior change)
- [ ] `ops::Session` + typed results; commands.rs becomes printers
- [ ] full suite green; no CLI output changes (e2e tests are the proof)

### Phase 1 — MCP server, tools
- [ ] `braid mcp` subcommand (stdio, rmcp), tool registry per the table
- [ ] typed input schemas; outputs are schema-conformant strand records
- [ ] rotation re-check per op; `synced` flag on mutation results
- [ ] `--read-only`; `--project`; server-level identity
- [ ] e2e stdio harness (spawn, initialize, tools/list, tools/call)
- [ ] hygiene e2e: doc id absent from every tool result and error

### Phase 2 — docs
- [ ] README section + docs/mcp.md (host setup snippets: Claude Code,
      Claude Desktop)
- [ ] agents-info note (for shell agents: "an MCP variant exists; prefer
      the CLI when you have a shell")

### Phase 3 — resources & notifications
- [ ] `braid://ready`, `braid://strand/{id}`, `braid://skein` resources
- [ ] changes() → resources/updated notifications; liveness e2e
- [ ] revisit strand priority/close

## Out of scope (recorded, not planned)

- Network transports (SSE/HTTP) — would reopen the capability-exposure
  problem; revisit only with real authentication.
- Prompts (`prompts/list`) — low value until a concrete host use case.
- Per-call impersonation/multi-user identity.
