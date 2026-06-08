# Windows support: CI, release artifact, easy install

Strands: **br-9b5rxe75** (Windows support pass — code portability) and
**br-jrt1n7pl** (Windows release artifacts + installer). Branch:
`windows-support`.

## Goal & sequence (agreed with Carlos)

Neither maintainer has a Windows machine, so **CI on `windows-latest` is the
only validator**. That makes "get Windows into CI" both the portability
proof and the artifact source.

1. **CI build + test on `x86_64-pc-windows-msvc`** — prove the code works;
   closes most of br-9b5rxe75.
2. **Windows `.zip` artifact in `release.yml`** — the foundation every
   install channel consumes.
3. **Easy install** — Scoop manifest + PowerShell one-liner (mirrors the
   `curl | bash` UX). (WinGet + ARM64 deferred.)

beads_rust (in `external-sources/`) is a working reference for all three
(it builds `x86_64-pc-windows-msvc`, ships a `.zip`, and maintains a Scoop
manifest auto-updated on release).

## Phase 1 — CI (in progress)

### Portability audit (2026-06-08)

- **Hard bug:** cache (`cache.rs`) and user config (`config.rs`) resolved
  the home dir from `HOME` only; Windows uses `USERPROFILE`. Fixed: fall
  back `HOME` → `USERPROFILE` (XDG still wins when set). Unit tests added
  in `tests/cache.rs` and `tests/config.rs`.
- **`tests/installer.rs` is Unix-only** (runs `bash install.sh`, writes
  `0o755` `uname` shims, `use std::os::unix::fs::PermissionsExt` at file
  top). Gated with a top-of-file `#![cfg(unix)]` — correct, it tests a bash
  script; the Windows installer gets its own tests in Phase 3.
- **`cfg(unix)` permission tightening** (cache 0o700, secret 0o600, hook
  0o755 in xtask) degrades to a no-op on Windows — acceptable for now;
  Windows ACL hardening is a separate, later concern. The `#[cfg(unix)]`
  permission assertion in `tests/cli.rs` compiles out cleanly.
- **Suspected, pending CI signal:** every e2e harness uses `.env_clear()`
  then sets only `PATH` + `HOME`. On Windows `env_clear` strips
  `SystemRoot`/`SystemDrive`, which the spawned binary likely needs (DLL
  load, DNS, TLS). If the first Windows run fails broadly at spawn/connect,
  the fix is to preserve those vars (and set `HOME`/`USERPROFILE`) in the
  harnesses — done systematically once the actual error confirms it.

### Work items

- [x] `HOME` → `USERPROFILE` fallback in `cache_dir` and `user_config_path`
      (+ unit tests).
- [x] Gate `tests/installer.rs` to `#![cfg(unix)]`.
- [x] Add `windows-latest` job to `ci.yml` (build + test + clippy; no
      minisign needed since installer tests compile out).
- [ ] First Windows CI run green — iterate on real failures (env_clear /
      SystemRoot the prime suspect; possibly path/line-ending assertions).
- [ ] Update br-9b5rxe75 with what CI actually surfaced.

## Phase 2 — release artifact (not started)

- [ ] Add `x86_64-pc-windows-msvc` to the `release.yml` build matrix; the
      runner is `windows-latest`. Package `braid.exe` as
      `braid-<version>-windows_amd64.zip` (zip is the Windows convention and
      what Scoop/WinGet expect). Tar packaging on the unix targets stays.
- [ ] Sign + checksum like the other artifacts (minisign Ed25519 runs on
      Windows; or sign in the combine job). Update the release notes table
      and the asset-count check (currently asserts 13 assets → becomes the
      4-tarball + 1-zip set with their `.sha256`/`.minisig`).
- [ ] Keep the artifact-naming contract aligned with `install.sh` and any
      Windows installer.

## Phase 3 — easy install (not started)

- [ ] **Scoop manifest** (`packaging/scoop/braid.json`) pointing at the
      release `.zip` + sha256; host in a bucket (e.g. `cscheid/scoop-braid`)
      and auto-update on release (beads' `update-package-manifests.yml` is
      the model). Add a test analogous to `tests/installer.rs` if feasible.
- [ ] **PowerShell installer** (`install.ps1`): resolve latest release,
      download the zip, verify SHA256 (`Get-FileHash`), extract, add to
      PATH. minisign verification optional (baseline is SHA256).
- [ ] Docs: README install section + a note in `agents-info.md`.

Deferred: WinGet manifest (broadest reach, later); `aarch64-pc-windows-msvc`
(low demand); Authenticode code-signing (needs a paid cert — document the
SmartScreen expectation instead).
