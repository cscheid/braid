# braid MCP server

`braid mcp` serves a skein to MCP hosts over stdio — for agents that have
no shell (Claude Desktop, claude.ai connectors via a local proxy, IDE
assistants) or that you deliberately don't *give* a shell.

## Why MCP instead of the CLI + skill?

For shell-capable agents, the CLI + `braid agents-info` skill remains the
preferred interface. The MCP server adds three things the CLI cannot:

1. **Shell-less agents** get a door into the skein.
2. **Capability isolation**: the server process holds the skein secret;
   the agent calls tools and *never possesses the doc id*. There is no
   `.braid.toml` for it to read and no `braid secret` for it to be talked
   into running — you can't leak what you never see. (The e2e suite
   asserts the doc id appears in no protocol message.)
3. **A long-lived session**: no per-command dial latency, continuous
   sync, and (coming) change notifications.

## Host setup

The server reads the normal braid configuration (env >
`.braid.toml` walk-up > user config) from its working directory, or from
`--project <dir>`.

**Claude Code** (`.mcp.json` in the project, or `claude mcp add`):

```json
{
  "mcpServers": {
    "braid": {
      "command": "braid",
      "args": ["mcp"],
      "env": { "BRAID_AUTHOR": "claude" }
    }
  }
}
```

**Claude Desktop** (`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "braid": {
      "command": "/path/to/braid",
      "args": ["mcp", "--project", "/path/to/your/project"],
      "env": { "BRAID_AUTHOR": "desktop" }
    }
  }
}
```

Identity is server-level (`BRAID_AUTHOR`, falling back through the usual
chain); there is deliberately no per-call author parameter.

## Capability tiers

| launch | tools served |
|---|---|
| `braid mcp --read-only` | queries only: ready, blocked, list, show, search, dep_list, dep_cycles, export |
| `braid mcp` (default) | queries + reversible mutations: create, update, close, reopen, defer, undefer, comment, dep_add, dep_remove |
| `braid mcp --enable-destructive` | everything + `braid_delete` and `braid_import` (**no undo**: a delete wins over concurrent edits; import overwrites same-id strands) |

Gating is enforced at call time as well as in `tools/list`: a tool the
server was not launched to serve refuses even if called by name. `secret`,
`init`, and `rotate` are **never** tools under any flag — they are
operator decisions made at a shell.

Every tool carries honest MCP annotations (`readOnlyHint`,
`destructiveHint`, `idempotentHint`, `openWorldHint: false`) so hosts can
build confirmation UX; annotations are hints, the launch flags are the
enforcement.

## Semantics worth knowing

- Tool outputs are `structuredContent`; strand records conform to
  [the published JSON Schema](schemas/strand.schema.json).
- `braid_list` and `braid_ready` accept optional field filters: `labels`
  (array — a strand must carry **all** of them), `assignee` (exact match;
  unassigned strands never match), and `type`. `braid_list` additionally
  takes `status` / `all`, mirroring the CLI flags.
- Mutation results carry `sync: "confirmed" | "unconfirmed" | "offline"` —
  whether the sync server acknowledged the change. Offline keeps working;
  results tell the truth. There is no sync tool: the session syncs
  continuously.
- If the skein is **rotated** while the server runs, every tool starts
  failing with the rotation message. Adoption is an operator action at a
  shell (`braid rotate --adopt`); restart the server afterwards.
- stdout is the protocol channel; diagnostics go to stderr.
