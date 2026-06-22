# How syncing works

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
