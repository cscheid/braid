---
name: braid-issue-tracking
description: Use braid to track issues, tasks, and bugs for this project. Trigger when filing, listing, updating, closing, or looking for work items, or when the user mentions braid issues.
---

braid is the project's distributed issue tracker (and this repo is also
braid's own source code — we dogfood). Run `braid agents-info` for the
full, version-matched guide. Quick start: `braid ready --json` lists
work; `braid create <title>` files new work; `braid close <id> --reason
<r>` completes it.

Set `BRAID_AUTHOR` to attribute your changes (e.g. `BRAID_AUTHOR=claude`).
The tracker secret lives in the gitignored `.braid.toml` at the repo root —
never commit it or paste the doc id anywhere.
