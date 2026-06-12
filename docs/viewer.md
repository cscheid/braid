# braid-viewer — desktop app

`braid-viewer` is a native desktop app built with [Tauri v2](https://tauri.app)
that wraps the braid React UI. It lets you open multiple braid project folders,
switch between skeins, and sync directly to your sync server — no CLI or local
server required.

## Running / building

```sh
# Development (hot-reload; requires tauri-cli and Node.js)
cargo xtask viewer-dev

# Release app (Tauri bundle — .app/.exe/.AppImage). The only command that
# produces a *runnable* standalone app.
cargo xtask viewer-build
```

Install `tauri-cli` once:
```sh
cargo install tauri-cli --version '^2' --locked
```

The viewer's frontend (project chooser, switching, add/remove) has React
component tests under `ui/src/*.test.tsx` (vitest + Testing Library, with Tauri
and automerge mocked). Run them with:
```sh
cargo xtask test-ui     # or: cd ui && npm run test
```
They also run as part of `cargo xtask ci` and in `ci.yml`.

> **Do not** run `cargo build --release -p braid-viewer` to launch the app.
> In Tauri v2 a runnable binary requires the `custom-protocol` feature, which
> only the Tauri CLI (`cargo tauri build`/`cargo xtask viewer-build`) sets. A
> plain `cargo build` omits it, so the binary starts in **dev mode** and tries
> to load the Vite dev server (`http://localhost:5173`); with no dev server
> running you get a blank `ERR_CONNECTION_REFUSED` window. `cargo build -p
> braid-viewer` is therefore only a **compile smoke check** (what `viewer.yml`
> CI runs) — not a way to produce a usable app.

### Logs

The app logs to stdout and to a rotating `braid-viewer.log` in the platform log
directory:

- **Linux:** `~/.local/share/org.cscheid.braidviewer/logs/`
- **macOS:** `~/Library/Logs/org.cscheid.braidviewer/`
- **Windows:** `%APPDATA%\org.cscheid.braidviewer\logs\`

A startup line records each window's resolved URL — if it shows
`http://localhost:5173` you built a dev binary (see the warning above). Because
the release exe runs with no console on Windows, this file is the primary way to
debug a launch failure. Logs are also forwarded to the webview console
(right-click → **Inspect**; available in release because the `devtools` feature
is enabled).

## Per-OS prerequisites

### Linux (Ubuntu / Debian)
```sh
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev librsvg2-dev \
  libsoup-3.0-dev build-essential
```
Minimum supported: **Ubuntu 22.04** (ships WebKitGTK 4.1).

### macOS
Xcode Command Line Tools (`xcode-select --install`). No extra packages needed.

First launch on macOS may show a quarantine dialog — right-click → Open to
bypass, or:
```sh
xattr -dr com.apple.quarantine target/release/braid-viewer
```

### Windows
MSVC toolchain + WebView2 runtime (ships with Windows 10 1803+ / Windows 11).
Build with `x86_64-pc-windows-msvc`.

## Adding projects

1. Launch `braid-viewer`.
2. Click **+ Add project** and pick a folder that contains a `.braid.toml`
   with a `doc_id`.
3. The viewer connects to the sync server declared in that folder's
   `.braid.toml` (or `wss://sync.automerge.org` by default).

The viewer remembers the last-opened project across restarts. Project paths
are stored in `~/.config/braid/viewer.toml` — **paths only, never secrets**.

## Switching / removing projects

- **Switch:** click **⇄ Projects** in the header to return to the chooser,
  then pick another registered project (or add one). Switching shuts down the
  previous skein's sync connection before opening the next, so only the active
  project syncs.
- **Remove:** in the chooser, click the **×** next to a project to drop it from
  the list (`viewer.toml`). This only forgets the path — it never touches the
  folder or its `.braid.toml`. Removing the project you're currently in returns
  you to the chooser.

## Offline / warm start

The viewer uses IndexedDB (via `@automerge/automerge-repo-storage-indexeddb`)
to cache each project's skein locally, namespaced by folder path. Cached data
persists across restarts and loads while the sync server is unreachable.

> **Note:** IndexedDB persistence under WebKitGTK (`tauri://` scheme on Linux)
> is empirically verified per OS — confirm on each target before relying on it
> in production.

## Custom sync servers (`allowed_sync_servers`)

By default the CSP allows `wss://sync.automerge.org`. To permit additional
servers, add them to `~/.config/braid/viewer.toml`:

```toml
allowed_sync_servers = ["wss://my-server.example.com"]
```

These are picked up at startup and merged into the CSP allowlist.

## Architecture

```
braid-viewer window
  └── Webview: ui/ React app
        ├── isTauri() → true → ViewerShell
        │     ├── invoke("list_projects_cmd")
        │     ├── invoke("add_project_cmd", { folder })
        │     └── invoke("get_config_cmd", { folder }) → docUrl + syncServer
        └── automerge-repo (WASM) ↔ wss://sync-server (direct, no Rust relay)
              └── IndexedDBStorageAdapter (namespaced per project)

Rust backend (braid-viewer): thin Tauri shell
  └── depends on braid-config (lean: no tokio/axum/samod)
        ├── viewer.rs  — project registry (viewer.toml, paths only)
        └── ui_config.rs — reads <folder>/.braid.toml, never walks up
```

Switching projects calls `repo.shutdown()` on the previous repo before
creating a new one, so only the active skein's WebSocket stays open.
