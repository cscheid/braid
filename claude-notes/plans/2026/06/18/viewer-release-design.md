# Design: ship braid-viewer bundles in each release

**Date:** 2026-06-18
**Status:** design approved; implementation plan to follow (writing-plans)
**Related strand:** signing follow-up `bd-ija7ely5`

## Overview

Today `release.yml` builds and publishes only the `braid` CLI (5 targets,
checksummed + minisign-signed, consumed by `install.sh` / `install.ps1`).
The `braid-viewer` Tauri desktop app (shipped in #13) has CI smoke checks
(`viewer.yml`) but is never released.

Goal: every `v*` release tag also attaches **braid-viewer GUI bundles**
(unsigned, v1) for macOS, Windows, and Linux, in the **same** GitHub
Release as the CLI artifacts.

The viewer is a separate install path from the CLI: `install.sh` /
`install.ps1` will **not** install it, and it is **not** part of the
minisign / checksum contract those scripts rely on.

## Key facts driving the design

- The viewer is a **Tauri v2 app**, not a plain binary. A *working*
  release build requires `cargo tauri build` (enables the `custom-protocol`
  feature and embeds the frontend); plain `cargo build -p braid-viewer`
  produces a dev-mode binary that loads `localhost:5173` and shows a blank
  window. (See `crates/braid-viewer/CLAUDE.md`.)
- `cargo tauri build` produces the platform **bundles** (.dmg / .nsis /
  .deb / .AppImage) as a byproduct of the same run — bundles are "free"
  relative to a raw-executable build. We ship bundles, the natural GUI
  distribution format.
- The repo already wraps this as `cargo xtask viewer-build`
  (`crates/xtask/src/main.rs`), which preflights for `tauri-cli` and
  `ui/node_modules`. We reuse it in CI.
- The `ui/` frontend (React/Vite, `ui/package.json`) is built by
  `tauri.conf.json`'s `beforeBuildCommand` (`npm run build` in `ui/`)
  during the tauri build, provided Node + `npm ci` ran first.
- The CLI release matrix ships **linux arm64** (musl static); the viewer
  matches for parity, but as glibc (Tauri cannot target musl).

## Decisions

| Question | Decision |
|---|---|
| Artifact | Platform bundles (not raw executables) |
| Workflow shape | **Inline** job in `release.yml` (not `workflow_call`, not a separate tag-triggered workflow) |
| Build command | `cargo tauri build` per target, via an extended `cargo xtask viewer-build` (see note) |
| Windows bundle | NSIS (`-setup.exe`) |
| Linux arm64 | Yes — parity with CLI |
| Signing | **Unsigned** for v1; tracked as follow-up `bd-ija7ely5` |
| Checksums | Ship a `SHA256SUMS-viewer` for manual verification (no minisign) |

### Why inline, not `workflow_call` or a separate workflow

- **Not `workflow_call`:** the viewer's PR/push smoke build
  (`cargo build -p braid-viewer`, a compile check in `viewer.yml`) and its
  release build (`cargo tauri build` + Node + bundling, a different matrix)
  are *different builds* — there is little to reuse. A reusable workflow
  would be mostly mode branches.
- **Not a separate tag-triggered workflow** (the approach
  `kenn-io/agentsview` uses): that pattern relies on
  `softprops/action-gh-release` create-or-append racing assets onto the
  release from two independent workflows. braid already has a single,
  deterministic `release` job doing `gh release create`; extending it is
  race-free and keeps one release-creation point. agentsview split only
  because its CLI is a separate Go/PyPI toolchain — braid's CLI and viewer
  share one cargo workspace.
- `viewer.yml` stays unchanged for PR/push smoke checks.

## Architecture

```
release.yml
  preflight ──┬─> build (CLI matrix, 5 targets)        ─┐
              └─> viewer (viewer matrix, 5 targets) NEW ─┤
                                                         ├─> release
                                                            (single gh release create;
                                                             asset glob includes viewer bundles)
```

### New `viewer` job

- `needs: preflight`, parallel to `build`.
- `strategy.matrix` (5 targets):

  | Platform | Runner | Target | Bundle(s) |
  |---|---|---|---|
  | macOS arm64 | macos-15 | aarch64-apple-darwin | `.dmg` |
  | macOS x86_64 | macos-15 | x86_64-apple-darwin | `.dmg` |
  | Windows x86_64 | windows-latest | x86_64-pc-windows-msvc | `.nsis` (`-setup.exe`) |
  | Linux x86_64 | ubuntu-22.04 | x86_64-unknown-linux-gnu | `.AppImage` + `.deb` |
  | Linux arm64 | ubuntu-24.04-arm | aarch64-unknown-linux-gnu | `.AppImage` + `.deb` |

- Steps per target:
  1. checkout at the tag ref (same as `build`).
  2. `dtolnay/rust-toolchain@stable` with the matrix target.
  3. `actions/setup-node` + `npm ci` in `ui/`.
  4. `cargo install tauri-cli --version '^2' --locked` (cached).
  5. Linux only: install WebKitGTK deps (same apt list as `viewer.yml`;
     `ubuntu-22.04` is the WebKitGTK LTS floor).
  6. `Swatinem/rust-cache` keyed per target.
  7. Build the bundle for the matrix target, restricting `--bundles` per
     OS to the subset above so `"targets": "all"` does not emit unwanted
     `.app`/`.rpm`/`.msi`. **Tooling note:** `cargo xtask viewer-build`
     today runs a bare `cargo tauri build` with no arg passthrough and no
     `--target` (`crates/xtask/src/main.rs`, `viewer_tauri`), so it cannot
     do per-target or bundle-restricted builds as-is. Preferred fix: extend
     `viewer-build` to forward trailing args (`cargo xtask viewer-build --
     --target <triple> --bundles <kind>`), keeping one build path shared by
     local + CI. Fallback: call `cargo tauri build --target ... --bundles
     ...` directly in CI. Decide in the plan; preferred = extend xtask.
  8. Verify the produced bundle's version matches
     `needs.preflight.outputs.version` (guards the version-drift fix).
  9. Collect bundles into a staging dir; compute per-file SHA-256.
  10. `actions/upload-artifact` (name `braid-viewer-<platform>`).

### `release` job changes

- Download viewer artifacts (already uses `download-artifact` with
  `merge-multiple`).
- Extend the "validate all platforms present" list to require the viewer
  bundles.
- Combine viewer checksums into `SHA256SUMS-viewer`.
- Add viewer bundles + `SHA256SUMS-viewer` to the `gh release create`
  asset list.
- Extend the release-notes body with a viewer-downloads table and the
  unsigned-install note.

## Version drift fix (required)

`crates/braid-viewer/tauri.conf.json` hardcodes `"version": "0.4.0"` while
the crate uses `version.workspace = true`. At release the bundle filenames
would carry `0.4.0`, not the tag. Fix: **remove the hardcoded `version`
line** from `tauri.conf.json` so Tauri inherits the crate (workspace)
version. The per-target version check in the `viewer` job (step 8) guards
against regressions.

## Docs (land in the same commit, per repo discipline)

- Release-notes generation in `release.yml`: add a viewer-downloads table
  row per bundle and an unsigned-install note:
  - macOS: first launch → right-click → Open (Gatekeeper quarantine), or
    `xattr -d com.apple.quarantine <app>`.
  - Windows: a browser-downloaded `-setup.exe` is unsigned → SmartScreen
    "More info → Run anyway". (Scoop/WinGet installs do not trip
    SmartScreen — no Mark-of-the-Web.)
- README: add a viewer-download mention if warranted.
- No `agents-info.md` / `docs/mcp.md` change (no new CLI subcommand or MCP
  tool), so `docs_drift.rs` is unaffected.

## Signing (deferred — `bd-ija7ely5`)

v1 ships unsigned everywhere. Evidence informing this:

- `kenn-io/agentsview` (closest peer: CLI + Tauri desktop in one release)
  **notarizes macOS** (Apple Developer ID + App Store Connect API key →
  `notarytool`, staples DMG) but **leaves Windows unsigned**.
- agentsview's Windows `.exe` is confirmed `NotSigned`
  (`Get-AuthenticodeSignature`); no SmartScreen prompt on Chris's machine
  is the **Scoop install path** (no Mark-of-the-Web), not signing.
- agentsview's Ed25519 keys are **auto-updater** machinery; braid ships no
  updater, so no updater signing is needed.

Follow-up work: evaluate whether a Posit Apple Developer account is
available, then wire `APPLE_*` secrets into the viewer job so
`cargo tauri build` signs + notarizes the DMGs. Windows Authenticode is
optional / lower priority.

## Testing

Per repo TDD discipline, the implementation plan leads with tests:

- Version-drift guard: a test asserting the viewer bundle / `tauri.conf`
  version resolves to the workspace version (so the hardcoded `0.4.0`
  regression cannot return). Mirror the spirit of `release.yml`'s existing
  preflight tag/version check.
- The `viewer` job's in-CI step-8 version check is itself the release-time
  assertion that bundles carry the tag's version.
- Validate the full release pipeline on a throwaway prerelease tag (manual
  `workflow_dispatch`) before relying on it for a real release.

## Out of scope

- macOS / Windows code signing + notarization (`bd-ija7ely5`).
- A viewer Scoop / WinGet manifest (separate work; would let Scoop users
  sidestep SmartScreen entirely).
- Tauri auto-updater and its signing artifacts.
