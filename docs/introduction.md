# braid

A local-first issue tracker for LLM agents (and the humans they work
with).

braid stores a project's issues in a **skein**: a single
[automerge](https://automerge.org) CRDT document, synced through an
automerge sync server. A single issue is a **strand** (full vocabulary:
[terminology](terminology.md)). There is no git involvement and no daemon:
any number of agents across machines, branches, and worktrees can create,
edit, and close strands in parallel — replication and conflict resolution
come from the CRDT, not from merge tooling.

braid is heavily inspired by [beads](https://github.com/steveyegge/beads):
it borrows the issue shape, dependency types, and ready/blocked workflow,
while replacing the git-committed `issues.jsonl` + SQLite machinery with a
synced document. (That JSONL file still matters: `braid import` migrates
it.)

![The braid web UI: a stage view with status lanes and priority-coloured strand cards.](braid-ui.webp)

## Where to next

- New here? [Install braid](installation.md), then walk the
  [quick start](quick-start.md).
- Running it for real? Read [the document id is a secret](security.md) — the
  doc id is a bearer token — and point braid at your own
  [configuration](configuration.md).
- Wiring up an agent? Shell-capable agents use the CLI (`braid agents-info`);
  shell-less ones use the [MCP server](mcp.md).
- Coming from beads? See [migrating from beads](migrating.md).
