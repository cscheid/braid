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

If one of these is in place, every command just works. Otherwise:

- new skein: `braid init` (then commit nothing — see the warning below)
- existing skein: `braid init --join <doc-id>`

**The doc id is a secret.** It is a bearer token granting read *and* write
access to the skein. Never commit `.braid.toml`; keep it gitignored.
Never print the doc id into logs, PRs, or strand text. braid itself only
shows a redacted prefix; the full id is printed exclusively by
`braid secret` — run that only when a human explicitly asks for it (e.g.
to configure another machine).

## The agent workflow

```sh
braid ready --json            # what can be worked on right now?
braid update <id> --status in_progress --assignee <you>
# ... do the work; leave a trail:
braid comment <id> "found the root cause in foo.rs; fixing"
braid close <id> --reason "fixed in commit abc123"
```

File newly discovered work as you go:

```sh
new=$(braid create "Fix the frobnicator" \
    --description "It frobs when it should nicate." \
    --type bug --priority 1 --label frobnicator)
braid dep add "$new" <current-strand-id> --type discovered-from
```

## Command reference

| command | purpose |
|---|---|
| `braid ready [--json]` | active, unblocked strands — best starting point |
| `braid blocked [--json]` | active strands blocked by dependencies, with blockers |
| `braid list [--status S] [--json]` | all strands |
| `braid show <id> [--json]` | one strand (unique id fragments work: `braid show 6j42`) |
| `braid search <text> [--json]` | case-insensitive substring over titles, prose, labels, comments |
| `braid create <title> [flags]` | new strand; prints its id. Flags: `--description --type --priority --label --slug --assignee --json` |
| `braid update <id> [flags]` | change fields: `--title --description --design --acceptance-criteria --notes --status --priority --type --assignee --external-ref --add-label --remove-label`; empty string clears |
| `braid close <id>... [--reason R] [--force]` | close; refuses if open children unless `--force` |
| `braid reopen <id>...` | reopen closed strands |
| `braid comment <id> <text>` | append a comment |
| `braid dep add <id> <target> [--type T]` | `<id>` depends on `<target>`; default type `blocks` |
| `braid dep remove <id> <target> [--type T]` | remove dependency |
| `braid dep list <id>` | dependencies in both directions |
| `braid dep cycles` | report dependency cycles |
| `braid sync` | force a sync; fails when the server is unreachable |
| `braid secret` | print the full doc id + sync server (paste-ready TOML). **Grants read/write access** — only run when a human asks |
| `braid import <file>` | import strands from JSONL (beads or braid format) |
| `braid export` | all strands as JSONL on stdout (backup / grep surface; records conform to the published JSON Schema — see `docs/schemas/` in the braid repo) |
| `braid init [--name N] [--join ID] [--sync-server URL] [--print-only]` | create or adopt a skein |

Conventions:

- **statuses**: `open`, `in_progress`, `blocked`, `deferred`, `closed`
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
