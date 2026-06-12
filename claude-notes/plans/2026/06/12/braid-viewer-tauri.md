# Plan: braid-viewer — a Tauri desktop app for braid

## Context

`braid` is a Rust issue tracker (skein = automerge doc, strand = issue). PR #11
(merged here, `d238a69`) added a React 19 + Vite web UI in `ui/`, served by
`braid ui`. We want a native desktop app — **braid-viewer** — wrapping that UI
with [Tauri v2](https://github.com/tauri-apps/tauri), with multi-project support.

### Two data layers (drives the storage design)
- **braid CLI (Rust):** `samod` + local cache at `~/.cache/braid/` → offline/warm
  start *for the CLI*.
- **The web UI / our thin-shell viewer (webview):** a *separate* client
  (`@automerge/automerge-repo`, WASM) syncing **directly to the sync server**,
  currently persisting nothing. It does **not** use braid's Rust cache.

So the viewer's data path is **webview ↔ sync server** (Rust only supplies
config); warm-start/offline must be added at the **webview** level (IndexedDB).

## Decisions (with the user)
1. **Monorepo**, reusing `ui/`.
2. **Thin shell** — webview syncs directly; new Rust = a small command set. (Path B deferred.)
3. **Multi-project** — project = folder with `.braid.toml`; selector; **one skein synced at a time**.
4. **Cross-platform** — Linux/macOS/Windows.
5. **Offline in v1** — IndexedDB adapter + pinned `data_directory`; **gated on an early per-OS persistence check** (esp. WebKitGTK under `tauri://`).
6. **Distribution = bare unsigned executable** for v1; installers/signing deferred.
7. **CSP allowlist** — default `wss://sync.automerge.org`, extended at runtime from registered projects + `allowed_sync_servers` (feasible via the per-webview hook; static `wss:` fallback).
8. **Lean dependency** — extract `crates/braid-config`; the viewer depends on *that*, not the heavy `braid` crate.

**Outcome:** `cargo xtask viewer-dev` opens a native window; add project folders,
switch skeins, view/edit strands with warm starts. `cargo build --release -p
braid-viewer` → a runnable executable per OS.

## Architecture
```
┌────────────────────── braid-viewer window ─────────────────────────┐
│ Webview = ui/ React app (automerge-repo + WASM + IndexedDB@dataDir) │
│  project selector + "Add project"                                   │
│       │ invoke(list/add/get_config)             WebSocket │         │
└───────┼───────────────────────────────────────────────────┼────────┘
        ▼ (Tauri IPC)                                         ▼
 braid-viewer (Rust): commands + startup CSP allowlist   each project's sync server
        │  depends on ↓ (lean, no tokio/axum/samod)
   crates/braid-config: config + docid + viewer registry + ui_config
        ▲  (braid re-exports it; braid-core unchanged)
```
Single frontend change vs. the browser: call **Tauri commands** instead of
`fetch("/api/config")`, plus an **IndexedDB** adapter. `ui/` serves both `braid
ui` (web) and braid-viewer (`isTauri()`-detected).

### Multi-project shell (viewer only)
Registry of project folders in `~/.config/braid/viewer.toml` (**paths only — never
secrets**); a selector; native folder picker. Selecting resolves *that folder's*
`{docUrl, syncServer}` and renders the existing single-skein view. **Switching
rebuilds the Repo and explicitly `shutdown()`s the previous one** (a `key=`
remount alone does not close the WebSocket).

## Workspace layout
```
crates/braid-config/   # NEW lean crate: config.rs + docid.rs (moved from braid)
                       #   + viewer.rs (folder registry) + ui_config + doc_url helper
                       #   deps: serde, toml, thiserror (NO tokio/axum/samod/rust-embed)
crates/braid/          # re-exports braid-config (pub use) → braid::config/::docid stable
crates/braid-viewer/   # Tauri crate; depends on braid-config + tauri + tauri-plugin-dialog
  Cargo.toml build.rs(tauri_build) tauri.conf.json src/{main,lib}.rs capabilities/ icons/
```
`braid-viewer` is a workspace **member** but excluded from `default-members`, and
`xtask`/CI drop `--workspace` (Phase 5) so routine builds skip it purely by
config — no `--exclude` flags. (Tauri can't build on musl, so it must stay out of
the default build set.) **Why a member, not a separate/nested workspace:** to share
one `Cargo.lock` + `target/` (consistent dep versions, build `braid-config` and
shared deps once and reuse them) and edit/test across crates atomically. A nested
workspace gives total `--workspace`/IDE isolation but costs the shared cache, a
second lockfile, and double-compiling `braid-config` — reach for it only if the
default-build leakage ever bites.

## Implementation plan

### Phase 0 — Prerequisites
- [ ] Per-OS Tauri deps: **Linux** webkit2gtk-4.1/soup-3/rsvg2/appindicator + build-essential/openssl;
      **macOS** Xcode CLT; **Windows** MSVC + `x86_64-pc-windows-msvc` + WebView2 runtime.
- [ ] `cargo install tauri-cli --version "^2"`. Pin the four `tauri*`/`@tauri-apps/*`
      packages exact (bump together). Create+commit `app-icon.png` (≥1024) → `cargo tauri icon`.
- [ ] Install **agent-browser** (`npm i -g agent-browser && agent-browser install --with-deps`).
      Confirm network policy allows tauri crates + `@tauri-apps/*` npm + Chromium.

### Phase 1 — Refactor + tests first (TDD)
- [ ] Extract **`crates/braid-config`**: move `config.rs` + `docid.rs` out of `braid`;
      `braid` adds `pub use braid_config::{config, docid};` (public paths unchanged →
      existing config/secret_hygiene tests pass untouched). Add `braid-config` to
      `members` + `default-members`.
- [ ] In `braid-config`: `doc_url(doc_id) -> String` (the `automerge:` prefix at
      `ui.rs:47-49`), `UiConfig{doc_url,sync_server}`, and `ui_config(folder)` that
      **parses `<folder>/.braid.toml` directly** (FileConfig + `DEFAULT_SYNC_SERVER`) —
      **no walk-up, ignore `BRAID_*` env** (strict folder semantics; see Gotcha).
      Refactor `braid`'s web `config_handler` (`ui.rs:80`) to keep using `config::load`
      (web behavior unchanged) but share `doc_url()`.
- [ ] `braid-config::viewer` registry + tests: add/list/remove folders round-trip
      through `viewer.toml`; `add_project` requires a parseable `<folder>/.braid.toml`
      with a `doc_id`. **Secret-hygiene test** (mirror `secret_hygiene.rs`): `viewer.toml`
      contains no `doc_id`/`docUrl` substring.

### Phase 2 — braid-viewer Rust backend (thin)
- [ ] Commands in `src/lib.rs` over `braid_config::*`: `list_projects`, `add_project`,
      `remove_project`, `get_config(folder)`. **Never log `UiConfig`/`docUrl`.**
- [ ] Startup: compute `connect-src` allowlist = `wss://sync.automerge.org` ∪ registered
      projects' sync servers ∪ `viewer.toml allowed_sync_servers`; inject via
      `WebviewWindowBuilder::on_web_resource_request` (rewrites the `tauri://` doc CSP) —
      fallback: a custom uri-scheme protocol handler that sets the header, or static `wss:`.
      **Per-project parse errors isolated** (one bad `.braid.toml` must not break startup).
- [ ] Set `WebviewWindowBuilder::data_directory(<app-data dir>)` so IndexedDB persists
      deterministically; `src/main.rs` registers `tauri_plugin_dialog` + `invoke_handler![…]`.

### Phase 3 — Tauri scaffold
- [ ] `Cargo.toml` (tauri, tauri-build, tauri-plugin-dialog, **braid-config** path dep, serde);
      `build.rs` = `tauri_build::build()`; `tauri.conf.json`: one window,
      `identifier="org.cscheid.braidviewer"`, **Windows `useHttpsScheme` set once & kept stable**
      (changing it relocates webview storage).
- [ ] Static baseline CSP: `default-src 'self'; script-src 'self' 'wasm-unsafe-eval';
      connect-src 'self' ipc: http://ipc.localhost wss://sync.automerge.org` (+ runtime
      extras from Phase 2; documented `'unsafe-eval'` fallback for old WebKitGTK).
- [ ] `capabilities/default.json`: `core:default` + `dialog:allow-open` (verified). App
      commands need no capability (verified). Build hooks (object form, `cwd:"../../ui"`):
      `beforeDevCommand npm run dev`, `beforeBuildCommand npm run build`,
      `frontendDist:"../../ui/dist"`, `devUrl:"http://localhost:5173"`.

### Phase 4 — Frontend (reuse `ui/`)
- [ ] Add `@tauri-apps/api`, `@tauri-apps/plugin-dialog`,
      `@automerge/automerge-repo-storage-indexeddb` to `ui/package.json`.
- [ ] `App.tsx` branch on `isTauri()` (`@tauri-apps/api/core`, verified): **Web** keeps
      `fetch("/api/config")`; **Viewer** renders a project shell (`list_projects`, selector,
      "Add project" → dialog `open({directory:true})` → `add_project`; remember last-active).
- [ ] **Repo lifecycle** (also fixes today's leak at `App.tsx:35`): create Repo in a
      `useEffect` keyed on `activeProjectId` with an **abort/generation guard** so a
      late-resolving `shutdown()` can't clobber the new repo; cleanup awaits/chains
      `shutdown()`; add the **IndexedDB** adapter **namespaced per project** (drop `isEphemeral`).
      Feed `docUrl` into the unchanged `<ConnectedApp/>`.

### Phase 5 — Build & workflow (the load-bearing CI fix)
- [ ] Root `Cargo.toml`: `members += braid-config, braid-viewer`;
      `default-members = ["crates/braid-core","crates/braid-config","crates/braid","crates/xtask"]`
      (ergonomics only).
- [ ] **Config-only exclusion (no `--exclude` flags):** with `default-members` set,
      change `cargo xtask ci` `CI_STEPS` (`xtask/src/main.rs:28-30`) **and**
      `.github/workflows/ci.yml` test/windows/musl jobs (`:45-47,:66-68,:113-114`) to
      **drop `--workspace`** (bare `cargo build/test/clippy --all-targets`). At the virtual
      workspace root, bare cargo operates on `default-members`, so the viewer is skipped
      automatically — exactly what `default-members` is for. (`-p braid-viewer` + the
      dedicated viewer job still build it.) Full isolation from anything hardcoding
      `--workspace` (e.g. rust-analyzer) would need a nested workspace — only if it bites.
- [ ] Because the viewer depends on lean **braid-config** (not `braid`), building it does
      **not** trigger braid's `build.rs`/rust-embed → **no double UI build, no `SKIP_UI_BUILD`
      hack, leaner binary**. (Also harden `build.rs:62` `ensure_stub` to not overwrite a
      real `index.html`; fix the stale "dist is committed" comments at `build.rs:3-12`,
      `xtask:94-96`.)
- [ ] xtask `viewer-dev`→`cargo tauri dev`, `viewer-build`→`cargo tauri build`. v1
      deliverable = `cargo build --release -p braid-viewer` (bare executable). Verify vite
      assets resolve under `tauri://` (empirical gate; only env-gate `base:"./"` if 404s).

### Phase 6 — CI (lean)
- [ ] New `viewer` job: matrix ubuntu/macos/windows **+ ubuntu-22.04** (LTS WebKitGTK floor);
      install per-OS deps + tauri-cli; `cargo build -p braid-viewer` smoke. Existing jobs gain
      `--exclude braid-viewer` (above) → stay green, musl unaffected.
- [ ] **Defer** producing/attaching `.dmg/.msi/.AppImage` until the app runs on all 3 webviews.

### Phase 7 — Docs
- [ ] `docs/viewer.md` (per-OS run incl. one-time mac/Windows first-run bypass; min WebKitGTK;
      `allowed_sync_servers`; `cargo tauri icon`); README "braid-viewer (desktop)". No
      `docs_drift.rs` impact (own binary, not a subcommand/MCP tool).

### Phase 8 — Deferred
- [ ] Installers + macOS notarization + Windows signing. Project labels/reordering. Path B.

## Critical files
- Move/refactor: `crates/braid/src/config.rs` + `docid.rs` → `braid-config` (re-export);
  `ui.rs` (share `doc_url`, web path keeps `config::load`); `build.rs` + `xtask:94` (stale
  comments + `ensure_stub`); `App.tsx` (dual-mode + Repo lifecycle + IndexedDB); root
  `Cargo.toml`; `crates/xtask/src/main.rs:28-30` + `.github/workflows/ci.yml` (`--exclude`).
- New: `crates/braid-config/src/viewer.rs` (+ tests), all of `crates/braid-viewer/`,
  `ui/package.json`, `docs/viewer.md`.

## Verified against Tauri v2 source/docs (read-only)
- App commands need no capability; `isTauri()`/`invoke` in `@tauri-apps/api/core`;
  `dialog:allow-open` — confirmed.
- **`on_web_resource_request` is a per-webview `WebviewWindowBuilder` method in v2** (NOT on
  the global `Builder`; it was global in v1) and can rewrite the `tauri://` CSP;
  `register_uri_scheme_protocol` handlers can set headers; **`data_directory` exists** →
  runtime CSP + pinned storage are both feasible.
- **`--workspace` overrides `default-members`** → so the fix is to **drop `--workspace`** in
  CI/xtask and let `default-members` exclude the viewer (config-only), not to add `--exclude`
  (confirmed in `xtask`/`ci.yml`).
- Outbound `wss://` from the webview is allowed; `braid ui` already syncs from arbitrary
  origins → server doesn't gate on `Origin`; `wss` is secure → no mixed-content.
- **CSP is injected only into the bundle, not the dev server** → always verify the built executable.
- Automerge WASM already runs in `ui/` via `vite-plugin-wasm`; single-threaded (no COOP/COEP).
  WebKitGTK target is `webkit2gtk-4.1`. Final WASM+storage check is empirical per OS.

## Gotchas (from two adversarial reviews)
- `--workspace` ignores `default-members` → drop `--workspace` in CI/xtask and rely on
  `default-members` (config-only); `-p`/the viewer job build it explicitly.
- Lean `braid-config` dep removes the double-build / `SKIP_UI_BUILD` / bloat hazards.
- `key=` remount ≠ socket close → explicit `shutdown()` with an abort/generation guard (async race).
- `add_project` must parse the folder's **own** `.braid.toml` (no walk-up, no `BRAID_*` env),
  else a subfolder/ambient-env resolves the wrong skein.
- CSP runtime injection is per-webview (`on_web_resource_request`)/custom-protocol, not a config
  string; static `wss:` is the always-works fallback. Isolate per-project parse errors.
- IndexedDB persistence under `tauri://` is unproven per-engine → pin `data_directory` + empirical gate.
- Secret hygiene: `viewer.toml` = paths only; never log `docUrl`. Keep `useHttpsScheme` stable.

## Verification (end-to-end)
1. `cargo test -p braid-config -p braid` — `ui_config`, registry, viewer.toml hygiene; existing tests green.
2. `cargo xtask ci` (now without `--workspace`, driven by `default-members`) + the musl job stay green.
3. `cargo xtask viewer-dev` → add two project folders, switch, confirm each shows its own
   strands **and the previous socket closes** (only active syncs).
4. **Build the executable** (`cargo build --release -p braid-viewer`) and run it — CSP is only
   enforced here: WASM loads, `wss` syncs, an `allowed_sync_servers` entry takes effect. Quit &
   relaunch → **warm start from IndexedDB**; offline → reads work. Repeat per OS incl. **Ubuntu 22.04**.
5. agent-browser drives `braid ui` in a browser to verify UI logic (rendering/filters/edits).

> During implementation: track the work as a braid strand (`BRAID_AUTHOR=claude`),
> marking it `in_progress` and assigning it to `claude` when work starts.
