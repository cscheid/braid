# braid — distributed issue tracking for agents and humans

braid tracks work in a **skein**: the collection of all issues for a
project, stored as a single CRDT document (automerge) and synced through a
sync server. A single issue is a **strand**. There is no git involvement:
multiple agents on multiple machines, branches, or worktrees can create,
edit, and close strands in parallel; replication and conflict resolution
are automatic. Commands work offline against a local cache and sync
whenever the server is reachable.

This guide is printed by `braid agents-info` and always matches the
installed version of braid.

## Setup (usually already done)

braid finds its skein via, in order (first hit wins):

1. `BRAID_DOC_ID` / `BRAID_SYNC_URL` environment variables
2. a `.braid.toml` file in the current directory or any parent
3. `~/.config/braid/projects.toml`, selected by a committed
   `.braid-project` marker file naming a project

Run `braid config` to see what resolved and from which file/layer (the doc
id is redacted) — handy when a directory has no `.braid.toml` but braid
still works (its doc id is coming from the user config via a `.braid-project`
marker, or from the environment).

If one of these is in place, every command just works. Otherwise:

- new skein: `braid init` (then commit nothing — see the warning below)
- existing skein: `braid init --join <doc-id>`

**The doc id is a secret.** It is a bearer token granting read *and* write
access to the skein. Never commit `.braid.toml`; keep it gitignored.
Never print the doc id into logs, PRs, or strand text. braid itself only
shows a redacted prefix; the full id is printed exclusively by
`braid secret` — run that only when a human explicitly asks for it (e.g.
to configure another machine).

To wire braid into a project's agent tooling, run `braid agents-info
--install <dir>` (e.g. `.claude/skills/braid/`). It writes a `SKILL.md` —
YAML frontmatter (`name` from the directory, plus a `description` so the
skill auto-invokes) over a body that just defers back to `braid
agents-info`, so it never goes stale. The installer is idempotent: it
refreshes the braid-managed head in place and preserves any content after
the end marker; it refuses to overwrite a `SKILL.md` braid didn't write.

## The agent workflow

```sh
braid ready --json            # what can be worked on right now?
braid update <id> --status in_progress --assignee <you>
# ... do the work; leave a trail:
braid comment <id> "found the root cause in foo.rs; fixing"
braid close <id> --reason "fixed in commit abc123"
```

File newly discovered work as you go — `--deps` links it in one shot:

```sh
braid create "Fix the frobnicator" \
    --description "It frobs when it should nicate." \
    --type bug --priority 1 --label frobnicator \
    --deps discovered-from:<current-strand-id>
```

## Command reference

| command | purpose |
|---|---|
| `braid ready [--label L]... [--assignee A] [--type T] [--priority N]... [--json]` | active, unblocked strands — best starting point. Filters narrow the set: `--label` is repeatable (a strand must carry all), `--assignee` is an exact match, `--type` matches the issue type, `--priority` is repeatable 0..=4 (a strand matches any of the given priorities) |
| `braid blocked [--json]` | active strands blocked by dependencies, with blockers |
| `braid list [--status S] [--all] [--label L]... [--assignee A] [--type T] [--priority N]... [--json]` | open (non-closed) strands; `--all` includes closed. Same field filters as `ready` |
| `braid show <id> [--json]` | one strand (unique id fragments work: `braid show 6j42`) |
| `braid search <text> [--json]` | case-insensitive substring over titles, prose, labels, comments |
| `braid create <title> [flags]` | new strand; prints its id. Flags: `--description --type --priority --label --slug --assignee --deps --json`. `--deps <type>:<target-id>` attaches dependencies atomically (repeatable and comma-separated; the new strand depends on each target). A missing target fails the create, like `dep add` |
| `braid update <id> [flags]` | change fields: `--title --description --design --acceptance-criteria --notes --status --priority --type --assignee --external-ref --add-label --remove-label`; empty string clears |
| `braid close <id>... [--reason R] [--force]` | close; refuses if open children unless `--force` |
| `braid reopen <id>...` | reopen closed strands |
| `braid defer <id>... [--until W]` | park strands. `--until` takes RFC 3339 (`2026-07-01T09:00:00Z`), a date (`2026-07-01`), or a duration (`36h`, `7d`, `2w`); once it passes, the strand counts as ready again (no daemon — readers compute the wake). Without `--until` it sleeps until `undefer` |
| `braid undefer <id>...` | wake deferred strands now (status back to `open`) |
| `braid delete <id>... [--force]` | remove strands entirely. Prefer `close` — a delete wins over concurrent edits and cannot be undone; `--force` needed if other strands reference the target |
| `braid comment <id> <text>` | append a comment |
| `braid dep add <id> <target> [--type T]` | `<id>` depends on `<target>`; default type `blocks` |
| `braid dep remove <id> <target> [--type T]` | remove dependency |
| `braid dep list <id>` | dependencies in both directions |
| `braid dep tree <id> [--json]` | recursive parent-child descendant tree (epic → subtasks); each node shows status, closed children included, cycles broken with a `(cycle)` marker |
| `braid dep cycles` | report dependency cycles |
| `braid sync` | force a sync; fails when the server is unreachable |
| `braid config` | show the resolved doc id (redacted), sync server, and author, each with the file/layer it came from — diagnose which config braid is using. Safe to run; does not disclose the secret |
| `braid secret` | print the full doc id + sync server (paste-ready TOML). **Grants read/write access** — only run when a human asks |
| `braid rotate` | move the skein to a fresh document (sheds history); stale clones are told to `--adopt`. **Only run when a human asks** |
| `braid rotate --revoke` | rotation for a *leaked* doc id: no forwarding pointer is written; the new secret must be distributed out-of-band. **Only run when a human asks** |
| `braid rotate --adopt` | follow a rotation: switch this clone to the successor skein (stragglers written to `.braid-stragglers.jsonl` for review) |
| `braid import <file>` | import strands from JSONL (beads or braid format); beads tombstones (soft-deleted records) are recognized and skipped, reported as `(skipped N tombstones)` |
| `braid export` | all strands as JSONL on stdout (backup / grep surface; records conform to the published JSON Schema — see `docs/schemas/` in the braid repo) |
| `braid init [--name N] [--join ID] [--sync-server URL] [--print-only]` | create or adopt a skein |

Conventions:

- **statuses**: `open`, `in_progress`, `blocked`, `deferred`, `closed`.
  A `deferred` strand whose `defer_until` has passed shows up in `ready`
  with its status still reading `deferred` — pick it up normally (e.g.
  `update --status in_progress`); leaving `deferred` clears the wake time.
  Deferral does not release dependents: only `closed` unblocks them
- **types**: `task`, `bug`, `feature`, `epic`, `chore`, `docs`, `question`
- **priority**: `0` (critical) … `4` (backlog); default `2`
- **dependency types**: `blocks`, `conditional-blocks`, `waits-for` make a
  strand unready while the target is open; `parent-child` expresses
  hierarchy (children stay workable; the *parent* refuses to close while
  children are open); `related`, `discovered-from`, `replies-to`,
  `duplicates`, `supersedes`, `caused-by` are informational
- prefer `--json` output for machine consumption; ids look like
  `br-x7k2m9q4` or `br-my-slug-x7k2m9q4`

## Sync model

Every command loads the local cache, dials the configured sync server,
exchanges changes (bounded by `BRAID_SYNC_TIMEOUT` seconds, default 5),
and exits. If the server is unreachable you get a stderr warning and work
continues from the cache; the next successful command converges. Set
`BRAID_AUTHOR` to attribute your changes (falls back to git `user.name`,
then the OS username).

Concurrent edits merge automatically: edits to different fields both
survive; concurrent edits to the same prose field interleave
character-wise; same scalar field, last writer wins; deleting a strand
wins over concurrent edits to it.

If a command fails with "this skein was rotated": the project moved to a
successor document. Run `braid rotate --adopt` if the error suggests it;
otherwise ask a human for the new secret. Do not try to work around the
error — writes to a rotated skein are abandoned.

## MCP variant

`braid mcp` exposes these operations as MCP tools for harnesses without a
shell. If you are reading this, you have a shell — prefer the CLI; it has
the complete surface (the MCP server deliberately omits secret/init/
rotate, and gates delete/import behind a launch flag).

## Installing a braid skill ("tying the knot")

To teach an agent harness about braid, install a skill that defers to this
command — it stays correct as braid evolves:

```markdown
---
name: braid-issue-tracking
description: Use braid to track issues, tasks, and bugs for this project.
---

braid is the project's distributed issue tracker. Run `braid agents-info`
for the full, version-matched guide. Quick start: `braid ready --json`
lists work; `braid create <title>` files new work; `braid close <id>
--reason <r>` completes it.
```

Save it (e.g. `.claude/skills/braid-issue-tracking/SKILL.md` for Claude
Code) and the agent will pull the rest from `braid agents-info` on demand.
