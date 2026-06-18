# braid-viewer: roborev 1618 fixes (CSP + error display)

## Overview

Two findings from roborev review #1618 against the `claude/braid-viewer`
branch. Both are **desktop-viewer only** (web mode via `braid ui` is
unaffected).

1. **Medium — custom sync server CSP.** The packaged viewer's CSP
   (`tauri.conf.json`) only allows `wss://sync.automerge.org`. A project
   registered with a non-default `sync_server` loads but the webview blocks
   the WebSocket, so it cannot sync. The `_extra_servers` allowlist is
   computed in `lib.rs` setup but never applied.
2. **Low — `[object Object]` error display.** `ViewerCommandError` serializes
   as `{ kind, message }` (`#[serde(tag = "kind", content = "message")]`), but
   the UI renders Tauri `invoke` errors with `String(err)`, yielding
   `[object Object]` instead of the actionable message.

Mechanism for fix 1 (verified via Tauri v2 docs): `WebviewWindowBuilder`
exposes `.on_web_resource_request(|req, res| ...)` which fires for
`tauri://` responses and lets us mutate the `Content-Security-Policy`
header. Tauri's `Csp` helper parses/serializes the policy as a directive map.
The handler does **not** fire for the dev server (`localhost:5173`), which is
fine — the dev binary has no CSP enforcement to break.

**Design decision (runtime add):** the CSP allowlist is a **startup
snapshot** built from registered projects' `sync_server` plus
`allowed_sync_servers` in `viewer.toml`. A project added at runtime on a
non-default server needs a viewer restart (or pre-declaration in
`allowed_sync_servers`) before it can sync. Documented, not auto-handled.

## Phase 1 — Tests first (TDD)

- [ ] **UI test (fix 2):** in `ui/src/App.viewer.test.tsx`, add a case where
      `get_config_cmd` (or `add_project_cmd`) rejects with a structured
      `{ kind, message }` object. Assert the rendered error shows the
      `message` text and NOT `[object Object]`. Follow the existing
      mock-Tauri/mock-automerge pattern in that file. Confirm it FAILS against
      current `String(err)`.
- [ ] **Rust test (fix 1):** in `crates/braid-viewer` (or `braid-config` if the
      helper lands there), add a unit test for the CSP-building helper: given a
      base CSP string + a list of extra servers, the resulting `connect-src`
      contains the default server AND each extra server, deduped, and other
      directives are untouched. Confirm it FAILS before the helper exists.

## Phase 2 — Fix 2 (error display)

- [ ] Add `errorMessage(err: unknown): string` helper in `ui/src/` (extracts a
      string `message` field when present, else `String(err)`). Keep it
      ES2020-safe (`tsconfig.app.json` targets ES2020).
- [ ] Apply at the 3 Tauri invoke catch sites: `App.tsx` ~176, ~211, ~226.
- [ ] Web-path catch (`App.tsx` ~87) gets a real `Error` — convert for
      consistency only if it stays trivially correct; otherwise leave.
- [ ] Phase 1 UI test passes; `cargo xtask test-ui` green.

## Phase 3 — Fix 1 (dynamic CSP)

- [ ] Extract a CSP-building helper that takes the base policy + extra servers
      and returns the augmented policy (append extra servers to `connect-src`,
      dedupe, preserve other directives). Use Tauri's `Csp` /
      `CspDirectiveSources` helpers.
- [ ] In `lib.rs` setup: rename `_extra_servers` → `extra_servers` (it is now
      used). Attach `.on_web_resource_request(...)` to the
      `WebviewWindowBuilder`; for `tauri://` requests with a CSP header, mutate
      `connect-src` via the helper using the captured `extra_servers` snapshot.
- [ ] Keep the static `tauri.conf.json` CSP as-is (default-server base).
- [ ] Phase 1 Rust test passes; `cargo build -p braid-viewer` (compile check)
      green.

## Phase 4 — Docs (same commit as code)

- [ ] `crates/braid-viewer/CLAUDE.md`: update the "CSP `connect-src` allows only
      `wss://sync.automerge.org` / non-default sync servers aren't supported
      yet" gotcha to describe the new behavior + the startup-snapshot / restart
      limitation.
- [ ] `docs/viewer.md`: if it repeats the limitation, update it; document
      `allowed_sync_servers` as the pre-declaration escape hatch.
- [ ] `lib.rs` comment at the old `_extra_servers` site: drop the "Unused
      until runtime CSP injection lands" note (it now lands).

## Phase 5 — Verify

- [ ] `cargo xtask test-ui` (UI vitest).
- [ ] `cargo build -p braid-viewer` (viewer compile check — excluded from
      default-members; `cargo xtask ci` does NOT build it).
- [ ] `cargo xtask ci` (fmt/clippy/build/test for the rest of the workspace —
      braid-config helper if it lives there).
- [ ] Manual: not blocking, but a packaged `cargo xtask viewer-build` with a
      project on a non-default server is the only true end-to-end check of CSP.
      Note as a follow-up if not run.

## Dev workflow

- Branch: continue on `claude/braid-viewer` (these are review fixes for that
  branch's PR).
- Commit fixes + their docs together (documentation-discipline rule).
- Attribute braid strand work with `BRAID_AUTHOR=claude`.
- **Do not push** — per project CLAUDE.md, ask Carlos before any remote-changing
  git command. Land via the existing PR.
