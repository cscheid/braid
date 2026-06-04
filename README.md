# braid

An automerge-centric issue tracker for LLM agents (and the humans they work
with).

braid stores a project's issues in a **skein**: a single
[automerge](https://automerge.org) CRDT document, synced through an
automerge sync server. A single issue is a **strand** (full vocabulary:
[docs/terminology.md](docs/terminology.md)). There is no git involvement
and no daemon: any number of agents across machines, branches, and
worktrees can create, edit, and close strands in parallel — replication
and conflict resolution come from the CRDT, not from merge tooling.

braid is heavily inspired by [beads](https://github.com/steveyegge/beads):
it borrows the issue shape, dependency types, and ready/blocked workflow,
while replacing the git-committed `issues.jsonl` + SQLite machinery with a
synced document. (That JSONL file still matters: `braid import` migrates
it.)

## Installation

```sh
curl -fsSL https://raw.githubusercontent.com/cscheid/braid/main/install.sh | bash
```

Installs the latest release to `~/.local/bin` after verifying its
SHA-256 checksum **and its Ed25519 signature** (release archives are
signed with [minisign](https://jedisct1.github.io/minisign/), the tool
Zig signs its releases with). Signature verification is mandatory, so
the installer needs minisign present — `brew install minisign`,
`apt install minisign`, `dnf install minisign`, or `apk add minisign`
first. Prebuilt binaries cover Linux x86_64/ARM64 (statically linked —
works on any distro, Alpine included) and macOS Intel/Apple Silicon.
The installer never asks questions and never edits your shell config;
if `~/.local/bin` isn't on your PATH it tells you the line to add.

The release signing key (since v0.2.1; pinned in `install.sh`, which
ships from this branch — not from the release being verified):

```
RWSbWhSzVkkTRO4nFMzL/KyRs9oicbgy/2KPRK+o9hxznRYx9ZkHwwlN
```

To verify a manually downloaded archive:
`minisign -Vm braid-<version>-<platform>.tar.gz -P <key above>` — the
trusted comment should name exactly the file you downloaded. If the
signing key is ever rotated, the new key lands here and in `install.sh`
in the same commit; releases keep the signatures they shipped with.

Useful flags (pass after `bash -s --`):

```sh
# specific version, custom directory
curl -fsSL .../install.sh | bash -s -- --version v0.2.1 --dest ~/bin

# build from source instead (needs a Rust toolchain)
curl -fsSL .../install.sh | bash -s -- --from-source

# install without signature verification (not recommended; note the
# flag goes after `bash -s --`, not on bash itself)
curl -fsSL .../install.sh | bash -s -- --insecure-skip-signature

# remove an installed binary
curl -fsSL .../install.sh | bash -s -- --uninstall
```

Alternatively, with a Rust toolchain:

```sh
cargo install --git https://github.com/cscheid/braid braid
```

## Quick start

```sh
# in your project directory
braid init                  # creates a skein, writes .braid.toml
echo .braid.toml >> .gitignore

braid create "Fix the frobnicator" --type bug --priority 1
braid ready                 # what's workable right now
braid close br-x7k2m9q4 --reason "fixed"
```

On another machine / clone / worktree of the same project:

```sh
braid init --join <doc-id>  # paste the doc id from the first machine
braid list                  # open strands, fetched from the sync server
```

Agents: run `braid agents-info` for a complete, version-matched usage
guide (it also shows how to install a one-paragraph skill that defers to
it).

## ⚠️ The document id is a secret

The automerge document id is a **bearer token**: anyone who has it can
read *and write* your skein, forever. Treat it like a credential:

- never commit `.braid.toml` (gitignore it; `braid init` reminds you)
- never paste the doc id into issue text, logs, commits, or PRs
- the default sync server, `wss://sync.automerge.org`, is a **public
  community relay**: it stores your document unencrypted, and possession
  of the id is the only access control. Fine for experiments; for real
  work, run your own sync server and set `sync_server` accordingly.

(The automerge ecosystem's capability-based access control + E2EE work —
Keyhive/Beelay — will eventually improve this story upstream.)

## Configuration

braid resolves its skein per-field, first hit wins:

1. **Environment**: `BRAID_DOC_ID`, `BRAID_SYNC_URL`, `BRAID_AUTHOR`
2. **Repo file**: a gitignored `.braid.toml` in the current directory or
   any parent:

   ```toml
   doc_id = "4UfaPGzzySmw7Y1MR1VVXbfp4fgx"
   sync_server = "wss://sync.automerge.org"   # optional
   author = "alice"                            # optional
   ```

3. **User config**: `~/.config/braid/projects.toml`, selected by a
   *committed*, non-secret `.braid-project` marker file containing a
   project name — useful so fresh worktrees need zero per-worktree setup:

   ```toml
   # ~/.config/braid/projects.toml
   [projects.myproject]
   doc_id = "..."
   sync_server = "wss://sync.example.com"
   ```

Authorship (`created_by`, comment authors) resolves as `BRAID_AUTHOR` →
config `author` → `git config user.name` → OS username.

## How syncing works

Every command: load the local cache → apply the change → dial the **one**
configured sync server → exchange changes (bounded by
`BRAID_SYNC_TIMEOUT` seconds, default 5) → exit. If the server is
unreachable, the command warns on stderr and works from the cache; the
next successful sync converges. `braid sync` forces a round trip and
fails loudly when offline.

If per-command network latency bothers you, don't ask braid for a daemon —
it deliberately has none. Run a local automerge sync server (samod-based,
for instance), point braid at it (`sync_server = "ws://localhost:8080"`),
and let that server peer with the remote.

The local cache lives under `~/.cache/braid/` (override with
`BRAID_CACHE_DIR`), is keyed by SHA-256 of the doc id so the secret never
appears on disk outside your config, is shared by all clones and
worktrees, and is safe to delete at any time. `BRAID_NO_CACHE=1` runs
fully stateless (requires the server).

Merge semantics, briefly: edits to different fields of the same issue both
survive; concurrent edits to the same prose field (description, design,
notes, comments) interleave character-wise; same scalar field → last
writer wins; deleting an issue wins over concurrent edits to it.

## Rotation: history compaction and leak recovery

A skein's automerge document keeps full history forever, and its doc id
is an irrevocable capability. `braid rotate` addresses both: it exports
the current state into a **fresh document** (shedding history), marks the
old document rotated, and switches your `.braid.toml`. Stale clones get a
clear error and run `braid rotate --adopt` to follow; changes they made
after the cutover are detected and written to `.braid-stragglers.jsonl`
for review and re-import.

If the doc id has **leaked**, use `braid rotate --revoke`: identical
mechanics, except no forwarding pointer is written into the old document
(the attacker can read it — a pointer would hand them the new
capability). Distribute the new secret out-of-band with `braid secret`.

Honest limits: rotation protects *future* reads and writes. The old
document's history remains readable forever to anyone holding the old id;
revocation cannot un-leak the past.

## MCP server

`braid mcp` serves the skein to MCP hosts over stdio — for shell-less
agents (Claude Desktop, IDE assistants) or agents you deliberately
sandbox: the server holds the secret and the agent **never possesses the
doc id**. Three capability tiers (`--read-only` / default /
`--enable-destructive`), honest tool annotations, call-time enforcement.
Setup snippets and semantics: [docs/mcp.md](docs/mcp.md). Shell-capable
agents should prefer the CLI (`braid agents-info`).

## Migrating from beads

```sh
braid init --prefix bd      # keep your bd- issue ids consistent
braid import .beads/issues.jsonl
```

Import upserts by issue id and accepts both beads JSONL and braid's own
`braid export` output (which is also your backup / grep surface:
`braid export | grep -i crlf`). beads' integer comment ids are replaced
with collision-free string ids; beads-only fields (`source_repo`,
compaction machinery, etc.) are dropped.

Export records conform to a published JSON Schema —
[`docs/schemas/strand.schema.json`](docs/schemas/strand.schema.json) —
the contract for downstream tooling; see
[`docs/schemas/README.md`](docs/schemas/README.md) for import
tolerances and the deltas vs the automerge document shape.

## Development

```sh
cargo test --workspace      # 120+ tests, no network required
cargo clippy --workspace --all-targets
```

The workspace has two crates: `braid-core` (schema, automerge
hydrate/reconcile, ready/blocked logic — no I/O) and `braid` (CLI, config
discovery, cache, sync). Design decisions and phase history live in
`claude-notes/plans/2026/06/03/braid-design-kickoff.md`; vocabulary in
`docs/terminology.md`. This repo dogfoods braid — run `braid list` here
to see its own skein.
