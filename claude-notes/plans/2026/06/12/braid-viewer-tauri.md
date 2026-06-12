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
- [x] Per-OS Tauri deps: **Linux** webkit2gtk-4.1/soup-3/rsvg2/appindicator + build-essential/openssl;
      **macOS** Xcode CLT; **Windows** MSVC + `x86_64-pc-windows-msvc` + WebView2 runtime.
- [x] `cargo install tauri-cli --version "^2"`. Pin the four `tauri*`/`@tauri-apps/*`
      packages exact (bump together). Create+commit `app-icon.png` (≥1024) → `cargo tauri icon`.
      **Fix (this session):** stub icons (PNG renamed to .ico) replaced with proper multi-size
      ICO (16/32/48/64/128/256 px, correct ICONDIR header) — Windows RC.EXE RC2175 error resolved.
- [ ] Install **agent-browser** (`npm i -g agent-browser && agent-browser install --with-deps`).
      Confirm network policy allows tauri crates + `@tauri-apps/*` npm + Chromium.

### Phase 1 — Refactor + tests first (TDD)
- [x] Extract **`crates/braid-config`**: move `config.rs` + `docid.rs` out of `braid`;
      `braid` adds `pub use braid_config::{config, docid};` (public paths unchanged →
      existing config/secret_hygiene tests pass untouched). Add `braid-config` to
      `members` + `default-members`.
- [x] In `braid-config`: `doc_url_str(raw) -> String` (`ui_config.rs`), `UiConfig{doc_url,sync_server}`,
      and `ui_config(folder)` that **parses `<folder>/.braid.toml` directly** (FileConfig +
      `DEFAULT_SYNC_SERVER`) — **no walk-up, ignore `BRAID_*` env** (strict folder semantics).
- [x] `braid-config::viewer` registry **tests**: add/list/remove folders round-trip
      through `viewer.toml`; `add_project` requires a parseable `<folder>/.braid.toml`
      with a `doc_id`. **Secret-hygiene test**: `viewer.toml` contains no `doc_id`/`docUrl` substring.
      *(6 new unit tests in `viewer.rs`; all 25 braid-config tests pass.)*

### Phase 2 — braid-viewer Rust backend (thin)
- [x] Commands in `src/lib.rs` (`mod commands` submodule to avoid E0255): `list_projects_cmd`,
      `add_project_cmd`, `remove_project_cmd`, `get_config_cmd`. Never logs `UiConfig`/`docUrl`.
- [ ] **CSP runtime injection**: `_extra_servers` is computed at startup but not yet injected
      into the webview CSP via `on_web_resource_request`. For v1 the static `wss:` fallback
      in `tauri.conf.json` covers the default server; inject dynamically when multi-server matters.
- [x] Dialog plugin registered; `ConfigDir` state managed; commands wired into `invoke_handler!`.
- [ ] `WebviewWindowBuilder::data_directory(…)` not yet set — IndexedDB will use the default
      location. Set explicitly for deterministic persistence (needed before offline/warm-start works).

### Phase 3 — Tauri scaffold
- [x] `Cargo.toml` (tauri, tauri-build, tauri-plugin-dialog, braid-config, serde, time pinned to
      `=0.3.47` to avoid Rust 1.94 E0119 coherence conflict with tauri's blanket `From` impls).
- [x] `build.rs` = `tauri_build::build()`.
- [x] `tauri.conf.json`: one window (1200×800), `identifier="org.cscheid.braidviewer"`, static
      baseline CSP, build hooks (`cwd:"../../ui"`, `devUrl`, `frontendDist`).
- [x] `capabilities/default.json`: `core:default` + `dialog:allow-open`.

### Phase 4 — Frontend (reuse `ui/`)
- [x] Add `@tauri-apps/api`, `@tauri-apps/plugin-dialog`,
      `@automerge/automerge-repo-storage-indexeddb` to `ui/package.json`; `npm install`
      updates `package-lock.json`.
- [x] `App.tsx` branch on `isTauri()` (`@tauri-apps/api/core`): **Web** keeps
      `fetch("/api/config")`; **Viewer** renders a project shell (`list_projects`, selector,
      "Add project" → dialog `open({directory:true})` → `add_project`; remember last-active
      in localStorage).
- [x] **Repo lifecycle** (also fixes today's leak at `App.tsx:35`): create Repo in a
      `useEffect` keyed on `[activeFolder, projectKey]` with an **abort guard** (`alive`
      flag) so late-resolving async can't clobber the new repo; cleanup calls
      `repo.shutdown()`; IndexedDB adapter **namespaced per project** (`braid-proj-${folder}`).
      Feed `docUrl` into the unchanged `<ConnectedApp/>`.

### Phase 5 — Build & workflow (the load-bearing CI fix)
- [x] Root `Cargo.toml`: `members += braid-config, braid-viewer`;
      `default-members = ["crates/braid-core","crates/braid-config","crates/braid","crates/xtask"]`.
- [x] **Drop `--workspace` in CI/xtask**: `xtask/src/main.rs` `CI_STEPS` no longer uses
      `--workspace` for clippy/build/test; `.github/workflows/ci.yml` test/windows/musl jobs
      updated. `default-members` naturally excludes `braid-viewer`. Updated xtask cli.rs test.
- [x] xtask `viewer-dev`→`cargo tauri dev`, `viewer-build`→`cargo tauri build`. Both invoke
      `cargo tauri <sub>` in `crates/braid-viewer/` with a helpful install hint if tauri-cli
      is missing.

### Phase 6 — CI (lean)
- [x] Viewer job moved to **separate path-filtered workflow** (`.github/workflows/viewer.yml`):
      triggers only on changes to `crates/braid-viewer/`, `crates/braid-config/`, `Cargo.toml`,
      `Cargo.lock`; matrix ubuntu-latest/ubuntu-22.04/macos-latest/windows-latest; Linux installs
      WebKitGTK 4.1 deps; `cargo build -p braid-viewer` smoke. `ci.yml` untouched by viewer changes.
- [ ] **Defer** producing/attaching `.dmg/.msi/.AppImage` until the app runs on all 3 webviews.

### Phase 7 — Docs
- [x] `docs/viewer.md` (per-OS run incl. one-time mac/Windows first-run bypass; min WebKitGTK;
      `allowed_sync_servers`; architecture diagram); README "braid-viewer (desktop)" section +
      updated Development section. No `docs_drift.rs` impact (own binary, not a
      subcommand/MCP tool).

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
