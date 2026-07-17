# How syncing works

Every command: load the local cache → apply the change → dial the **one**
configured sync server → exchange changes (bounded by
`BRAID_SYNC_TIMEOUT` seconds, default 5) → exit. If the server is
unreachable, the command warns on stderr and works from the cache; the
next successful sync converges. `braid sync` forces a round trip and
fails loudly when offline.

If per-command network latency bothers you, don't ask braid for a daemon —
it deliberately has none. Run a local sync server instead (braid ships
one — see below) and point braid at it
(`sync_server = "ws://localhost:3030"`).

## Running your own sync server: `braid serve`

braid ships a standalone sync server — in braid's vocabulary, a
[**loom**](terminology.md) — for when the configured server is down, you
are on an isolated network, or you'd rather not relay a project's issues
through a public server:

```sh
braid serve --data-dir ~/.local/share/braid/loom   # persistent
braid serve --in-memory                            # ephemeral relay
```

The storage choice is deliberately explicit — there is no default, so you
always know whether the loom persists and where. With `--data-dir`,
skeins survive restarts; the directory is created mode 700 and storage
keys are hashed, so doc ids (bearer secrets) never appear on the loom's
disk. With `--in-memory`, everything is forgotten on exit.

The loom binds `127.0.0.1:3030` by default (`--host`, `--port`; port `0`
picks a free one) and prints the URL to point clients at:

```sh
BRAID_SYNC_URL=ws://127.0.0.1:3030 braid list      # per invocation
braid init --sync-server ws://127.0.0.1:3030       # per project, at init
```

or durably for an existing project, set
`sync_server = "ws://127.0.0.1:3030"` in `.braid.toml`.

Two things to know before exposing a loom beyond loopback:

- **`--host 0.0.0.0` is a trust decision.** The loom has no
  authentication, by design — like the public sync server, possession of
  a doc id grants read and write access to that document, and nothing
  else is required. Anyone who can reach the port can store and fetch
  skeins whose ids they know.
- **No TLS.** The loom speaks `ws://`. For `wss://` across untrusted
  networks, terminate TLS in a reverse proxy (Caddy, nginx) in front of
  it.

It runs in the foreground and shuts down cleanly on Ctrl-C or SIGTERM
(flushing pending writes), so a systemd unit or launchd job is all the
supervision it needs:

```ini
# ~/.config/systemd/user/braid-loom.service
[Service]
ExecStart=%h/.local/bin/braid serve --data-dir %h/.local/share/braid/loom
Restart=on-failure
[Install]
WantedBy=default.target
```

`wss://` connections trust the compiled-in Mozilla (webpki) roots **plus**
the system trust store, honoring the standard `SSL_CERT_FILE` /
`SSL_CERT_DIR` variables. So braid works out of the box on a bare static
binary *and* behind a TLS-terminating egress proxy (corporate MITM,
sandbox) — point `SSL_CERT_FILE` at the proxy's CA bundle. Without the
system store a proxy-issued certificate fails the dial with
`UnknownIssuer`.

The local cache lives under `~/.cache/braid/` (override with
`BRAID_CACHE_DIR`), is keyed by SHA-256 of the doc id so the secret never
appears on disk outside your config, is shared by all clones and
worktrees, and is safe to delete at any time. `BRAID_NO_CACHE=1` runs
fully stateless (requires the server).

Merge semantics, briefly: edits to different fields of the same issue both
survive; concurrent edits to the same prose field (description, design,
notes, comments) interleave character-wise; same scalar field → last
writer wins; deleting an issue wins over concurrent edits to it.
