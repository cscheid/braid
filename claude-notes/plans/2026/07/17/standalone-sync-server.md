# Standalone sync server inside braid (the loom)

**Strand:** `br-loom-3qm0ze53` · **GitHub:** [#22](https://github.com/cscheid/braid/issues/22) · **Status:** draft, iterating with Carlos

## Overview

Ship a subcommand that runs a standalone automerge sync server so braid
users can collaborate through localhost or a self-hosted machine when the
default server (`wss://sync.automerge.org`) is unreachable — as of
2026-07-17 it is in fact down, which is why this plan was written against a
locally-running sync server.

This is the **loom** reserved in `docs/terminology.md` ("a sync-server
peer — in particular a future samod-based local relay binary, where skeins
are exchanged"). The model is `q2 hub --no-project` from Quarto 2
(`external-sources/q2/crates/quarto-hub`): a samod repo + websocket
acceptor + filesystem storage, no auth, no document access policy beyond
the doc-id-as-bearer-capability model braid already has.

Braid already owns every ingredient:

- samod 0.10 ships the server side: `Repo::make_acceptor(url)` →
  `AcceptorHandle::accept(transport)` (vendored reference:
  `external-sources/samod/samod/src/acceptor_handle.rs`).
- `braid::ws::ws_transport` (`crates/braid/src/ws.rs:132`) is explicitly
  direction-agnostic — its module doc says it serves both the dialer and
  "the in-process accept loop in the sync e2e tests".
- `crates/braid/tests/sync.rs:32` (`TestServer::start_scheme`) is already a
  working in-process sync server: `TcpListener` →
  `tokio_tungstenite::accept_async` → `acceptor.accept(ws_transport(ws))`.
  The feature is essentially promoting that harness to a real subcommand
  with persistent storage and a CLI surface.
- `crates/braid/src/cache.rs` has a hardened samod `Storage` impl
  (`FsStorage`) a server can reuse.

## CLI surface (decided with Carlos, 2026-07-17)

```
braid serve (--data-dir <path> | --in-memory) [--host 127.0.0.1] [--port 3030]
```

- **Command name: `braid serve`** (decided). Terminology governs prose,
  not command names; docs and output call the running thing "a loom"
  ("loom listening on ws://127.0.0.1:3030").
- **Storage is an explicit, required choice** (decided): exactly one of
  `--data-dir <path>` (persist docs there) or `--in-memory` (ephemeral
  relay, nothing touches disk, restart forgets everything). No default —
  the user always knows whether the loom persists and where. Clap
  `ArgGroup(required = true)` enforces it with a clear error.
- **`--host`**: default `127.0.0.1` (loopback-only by default; exposing to
  a LAN is an explicit `--host 0.0.0.0`). Matches q2 hub.
- **`--port`**: default `3030` (decided; matches the conventional local
  automerge sync-server port). `--port 0` binds an ephemeral port; the
  bound address is always printed as a parseable line so tests (and
  scripts) can discover it.
- Runs in the foreground, prints one line per client connect/disconnect
  (peer id from `AcceptorEvent`), shuts down gracefully on ctrl-c.

Clients need no new features: `BRAID_SYNC_URL=ws://127.0.0.1:3030 braid …`
or `sync_server = "ws://…"` in `.braid.toml` / `braid init --sync-server`
already work today.

## Design decisions (proposed)

- **D-serve-1 — transport**: raw `TcpListener` +
  `tokio_tungstenite::accept_async` + `ws_transport`, exactly like the test
  harness. No axum for v1 (nothing but websockets to serve), and
  **not** samod's `tungstenite` feature — it drags in native-tls/OpenSSL and
  breaks static musl builds (strand `br-f3b18xoa`, `crates/braid/Cargo.toml`
  comments). A raw websocket accept ignores the request path, so
  `ws://host:port`, `ws://host:port/`, and `ws://host:port/ws` all work —
  the same compatibility q2 gets by registering both `/` and `/ws`.
- **D-serve-2 — storage**: reuse `cache::FsStorage` (atomic writes,
  retry-on-transient-IO) wrapped in `HashedKeyStorage`, rooted at the data
  dir — the loom's disk never holds doc-id bearer secrets in the clear.
  **Verified feasible** (2026-07-17): the only `LoadRange` issuers in
  samod-core are `document/load.rs:71-77`, using
  `StorageKey::snapshot_prefix`/`incremental_prefix`, and both put the doc
  id as the first key component (`samod-core/src/storage_key.rs:41-55`).
  No samod code path enumerates storage with an empty prefix, so
  `HashedKeyStorage`'s fail-closed empty-prefix behavior is never hit.
  Trade-off accepted: the on-disk layout is braid-specific, so a loom
  cannot adopt an existing automerge-repo/nodefs data dir (and vice versa).
- **D-serve-3 — announce policy**: `NeverAnnounce`, set explicitly. The
  doc id is the capability; the server must not volunteer documents to
  connected peers. **Required, not optional** (verified 2026-07-17): samod's
  builder defaults to `AlwaysAnnounce` (`samod/src/builder.rs:103-109`),
  which on a multi-tenant relay would announce every doc it is
  synchronizing to every newly connected peer — a cross-tenant doc-id
  leak. q2 hub sets `NeverAnnounce` for the same reason. (braid's client
  side keeps the default; announcing the skein to the loom on connect is
  exactly how the loom learns about it.)
- **D-serve-4 — no auth in v1**: same trust model as sync.automerge.org
  (possession of the doc id grants read/write). Loopback default binding
  keeps the accidental exposure surface small; docs state plainly that
  `--host 0.0.0.0` means anyone who can reach the port can relay/store
  docs, and that TLS termination belongs in a reverse proxy. (q2's OIDC
  layer is opt-in and its access policy is allow-all anyway — not worth
  porting for v1.)
- **D-serve-5 — no daemonizing**: foreground process, like `braid ui` and
  `braid mcp`. systemd/launchd/tmux is the user's business; docs show a
  one-line systemd unit example.

## Out of scope (v1) — file as follow-up strands

- `--peer <url>` upstream relaying (loom ↔ loom / loom ↔ sync.automerge.org
  federation; q2 has this, braid can wait until there's a concrete need).
- TLS termination in-process (`wss://` serving) — reverse-proxy territory.
- Auth of any kind.
- HTTP health/metrics endpoint.
- Hosting the `braid ui`/viewer from the same port.

## Work items

### Phase 1 — tests (TDD)

- [x] Resolve Q1–Q4 with Carlos; update this plan (2026-07-17: `braid
      serve`; port 3030; hashed storage if samod permits; storage mode is a
      required explicit flag, no default).
- [x] Verify samod 0.10 server-side behavior against D-serve-2/D-serve-3
      (empty-prefix `load_range` use; default announce policy) — done
      2026-07-17, findings recorded in D-serve-2/D-serve-3: hashed storage
      is safe; `NeverAnnounce` is mandatory (default is `AlwaysAnnounce`).
- [x] `crates/braid/tests/serve.rs`: e2e — spawn `braid serve --port 0
      --in-memory` via `tokio::process`, parse the printed `ws://` address,
      stand up two clones (reuse the `Clone_` pattern from
      `tests/sync.rs`), both directions converge
      (`two_clones_converge_through_the_loom`).
- [x] e2e — persistence: `braid serve --data-dir <tmp>`, sync a skein,
      kill the server, restart on the same dir, a **fresh** clone with an
      empty cache can pull the skein (`loom_persists_skeins_across_restart`;
      also asserts the doc id never appears in storage paths, per D-serve-2).
- [x] e2e — `--in-memory` restart forgets docs; a fresh clone fails loudly
      rather than seeing an empty skein (`in_memory_loom_forgets_on_restart`).
- [x] CLI arg tests (e2e via the real binary rather than unit tests —
      matches repo convention): exactly one of `--data-dir` / `--in-memory`
      required, both/neither are errors naming both flags; `--help`
      documents the 3030/127.0.0.1 defaults.
- [x] `docs_drift.rs` will fail until agents-info mentions the subcommand —
      that is Phase 3's forcing function; no new drift test needed.

### Phase 2 — implementation

- [x] `Cmd::Serve` clap variant in `crates/braid/src/main.rs` + dispatch
      (clap `ArgGroup` enforces the storage choice).
- [x] `crates/braid/src/serve.rs` (`pub mod serve` in lib.rs):
      repo with `NeverAnnounce`, bind, `make_acceptor`, per-connection
      handshake tasks feeding `acceptor.accept(ws_transport(ws))`,
      connection logging from `acceptor.events()`, graceful shutdown on
      ctrl-c **and SIGTERM** (`repo.stop()` flushes storage).
- [x] Print the bound `ws://host:port` URL on startup (stable, parseable,
      stdout; all other logging on stderr).
- [x] Storage-mode wiring: `--data-dir` → `cache::open_cache_storage`
      (mode-700 dir, `HashedKeyStorage<FsStorage>` — direct reuse),
      `--in-memory` → `InMemoryStorage`.

### Phase 3 — docs (same commit as the feature, per CLAUDE.md)

- [x] `crates/braid/src/agents-info.md`: command-reference row + a "Sync
      model" note (how to point clients at a loom).
- [x] README: short "Run your own sync server" section.
- [x] Extended `docs/syncing.md` with a "Running your own sync server:
      `braid serve`" section (explicit-storage rationale, D-serve-4
      security note, TLS-via-reverse-proxy, systemd example). No new page,
      so `docs/SUMMARY.md` needed no change; `docs/security.md` now links
      to the section.
- [x] `docs/terminology.md`: loom is no longer "reserved / not yet built";
      the row now points at `braid serve`.
- [x] ~~CHANGELOG entry~~ — dropped: this repo keeps no CHANGELOG file
      (the one spotted earlier belongs to vendored samod).
- [ ] Close the strand with an outcome comment; file follow-up strands for
      the out-of-scope list (at minimum `--peer` relaying).

## Decisions from Carlos (2026-07-17)

1. **Q1 — command name**: `braid serve`. Prose calls the server a loom.
2. **Q2 — default port**: 3030.
3. **Q3 — storage keys**: hashed (`HashedKeyStorage`) if the samod source
   check confirms server paths never enumerate storage with an empty
   prefix. The check passed (see D-serve-2), so hashed it is.
4. **Q4 — persistence**: no default. The user must pass exactly one of
   `--data-dir <path>` or `--in-memory`.
