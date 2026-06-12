# braid-viewer — desktop app

`braid-viewer` is a native desktop app built with [Tauri v2](https://tauri.app)
that wraps the braid React UI. It lets you open multiple braid project folders,
switch between skeins, and sync directly to your sync server — no CLI or local
server required.

## Running / building

```sh
# Development (hot-reload; requires tauri-cli and Node.js)
cargo xtask viewer-dev

# Release executable (Tauri bundle — .app/.exe/.AppImage)
cargo xtask viewer-build

# Or: bare executable smoke build (no bundle; fastest; what CI does)
cargo build --release -p braid-viewer
```

Install `tauri-cli` once:
```sh
cargo install tauri-cli --version '^2'
```

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
