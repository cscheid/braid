# Documentation website over GitHub Pages

## Overview

The README had grown into the full reference manual (~280 lines). This change
adds a proper multi-page documentation site — an [mdBook](https://rust-lang.github.io/mdBook/)
built from the existing `docs/` directory and published to GitHub Pages — and
trims the README to a short, impactful front page (what it is → screenshot →
install → quick start → safety note → links).

mdBook reads `docs/` as its source (`book.toml` `src = "docs"`) so the files
the contract tests depend on stay put: `docs/mcp.md` (read by
`docs_drift.rs`) and `docs/schemas/strand.schema.json` (read by
`schema_contract.rs`). No existing doc files move; existing cross-links are
already relative within `docs/` and render unchanged.

Published URL: <https://cscheid.github.io/braid/> (canonical repo per
`Cargo.toml`/`install.sh`).

## Work Items

- [x] Add SUMMARY-completeness drift test (`docs_drift.rs`) — every
      `docs/**/*.md` must be listed in `docs/SUMMARY.md` (TDD: written first)
- [x] Create chapter pages from README sections: `introduction`,
      `installation`, `quick-start`, `configuration`, `security`, `syncing`,
      `rotation`, `web-ui`, `migrating`, `development`
- [x] Create `docs/SUMMARY.md` (TOC: Getting started / Concepts / Interfaces /
      Reference; existing `terminology`, `mcp`, `viewer`, `schemas/README`
      wired in)
- [x] Create `book.toml` (root; `src = "docs"`, rust theme, edit links)
- [x] Add `.github/workflows/docs.yml` (mdBook build + official Pages deploy)
- [x] Add `book/` to `.gitignore`
- [x] Add `cargo xtask docs` / `docs-serve`
- [x] Trim `README.md` to the front-page shape
- [x] Verify locally: `mdbook build` clean, no broken links, full link scan
- [x] `cargo xtask ci` green (incl. new drift test + UI tests)
- [ ] Hand-off note: repo owner enables Pages (Settings → Pages → Source =
      "GitHub Actions"); README/site cross-links go live after first deploy

## Notes

- mdBook special-cases `README.md → index.html` only for SUMMARY chapters, not
  inline links: an inline `[…](schemas/README.md)` renders to a broken
  `schemas/README.html`. Fixed by linking to the directory (`schemas/`), which
  resolves on both mdBook (→ `schemas/index.html`) and GitHub (→ folder).
- mdBook pinned to v0.5.3 in the workflow.
- Could not file a dogfooding braid strand: `.braid.toml` is gitignored and
  absent in this checkout (no doc-id secret available here).
