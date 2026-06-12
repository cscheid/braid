# braid ui — local web UI for skein management

## Overview

Add `braid ui` — a command that serves a minimal React app locally, letting
users visually inspect, edit, and manage strands in a skein.

Architecture: the local Rust server is intentionally thin. It serves one
endpoint (`GET /api/config` → doc URL + sync server URL) and static files for
the React bundle. The browser then connects **directly** to the automerge sync
server via WebSocket — no traffic proxies through the Rust process. This keeps
the server trivial and all CRDT logic in the browser.

## Stack

- **React 19 + TypeScript** — UI
- **Vite** — bundler (builds to `ui/dist/`, committed to repo)
- **@automerge/automerge-repo** — CRDT document management
- **@automerge/automerge-repo-network-websocket** — WebSocket sync adapter
- **@automerge/automerge-repo-react-hooks** — `useDocument` hook
- **axum** — minimal local HTTP server (static files + `/api/config`)
- **rust-embed** — compile-time embedding of `ui/dist/` into the binary

## UX Design

- Two-panel layout: fixed 280px left sidebar + fluid detail panel
- Strands grouped by status: `in_progress` → `open` → `blocked` → `deferred` → `closed`
- Within each group: sorted by priority (0=critical first)
- Priority shown as colour-coded left border on each card
- Click a card → detail appears in right panel (no navigation/page load)
- Inline field editing — click any field to edit, saves on blur
- Real-time sync — changes from other clients appear via automerge subscription
- Connection status dot in header
- Collapsible status groups for managing long lists

## Work Items

- [x] Create plan file
- [x] Create React UI source in `ui/`
- [x] Build UI: `npm install && npm run build`
- [x] Add `axum` + `rust-embed` to Cargo workspace and braid crate
- [x] Create `crates/braid/src/ui.rs`
- [x] Wire `Ui` subcommand into `main.rs` and expose module in `lib.rs`
- [x] Update `agents-info.md` (docs_drift test requires every subcommand listed)
- [x] Add `cargo xtask build-ui` to xtask
- [x] Run `cargo xtask ci` — all 275 tests pass
- [ ] Register braid issue and close with summary

## Security notes

- `/api/config` is served only on `127.0.0.1` (loopback only, never `0.0.0.0`)
- The doc URL (bearer secret) is never placed in the browser's URL bar or
  localStorage — it is fetched from the local API on startup and held only in
  React state
- A future improvement: validate the `Origin` header on `/api/config` to
  refuse requests not coming from the same local tab
