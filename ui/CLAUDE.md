# ui — React app for braid (web + desktop)

This **single** React app serves two front ends from one bundle:

- **Web** — `braid ui` serves the embedded build; config via
  `fetch("/api/config")`; ephemeral repo (no local storage).
- **Desktop** — `braid-viewer` (Tauri) loads the same bundle; config via Tauri
  `invoke(...)`; IndexedDB-backed repo; multi-project.

`App.tsx` branches on **`isTauri()`**. Keep the split clean: viewer-only
features (project chooser, `⇄ Projects` switcher, add/remove) must **not** show
in web mode — gate them via props (e.g. `onSwitchProject` is passed only by the
viewer shell). Never put Tauri `invoke` calls on the web path.

## Build

- `ui/dist` is **gitignored**. `braid`'s `build.rs` runs `npm ci && npm run
  build` and rust-embed embeds the output into the `braid` binary (the Tauri
  build embeds it too). Run `cargo xtask build-ui` after UI changes for a fresh
  `dist`.
- `tsconfig.app.json` targets **ES2020** — newer runtime APIs
  (`Array.prototype.at`, etc.) fail `tsc -b`. Use ES2020-safe idioms or bump the
  target deliberately.
- `docUrl` is a bearer secret: hold it in React state only — never in
  localStorage or the URL bar.

## Tests (vitest)

- Run `cargo xtask test-ui` (or `npm run test`). They also run in
  `cargo xtask ci` and `ci.yml` (Linux).
- Component tests live in `src/*.test.tsx` with `src/test/setup.ts`; both are
  **excluded from the `tsc -b` production build** (see `tsconfig.app.json`).
- **Mock Tauri and automerge** (`@tauri-apps/*`, `@automerge/*`) so no native
  WASM loads in jsdom — copy the pattern in `App.viewer.test.tsx`.
