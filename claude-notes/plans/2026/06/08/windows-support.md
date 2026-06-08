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
- [x] First Windows CI run green. What CI actually surfaced (4 iterations):
      1. Whole suite passed except live-sync tests — confirming the
         `HOME`→`USERPROFILE` fix and installer cfg-gate, and that
         `env_clear` does NOT break plain subprocess spawning on Windows.
      2. `env_clear` *does* break **networked** subprocesses: it strips
         `SystemRoot`, so Winsock can't init and the spawned `braid` can't
         reach the in-process sync server. Fixed in the harnesses that dial
         a live relay (`mcp_cli.rs`, `rotate.rs`, `sync.rs`) by re-adding
         `SystemRoot`/`SystemDrive`/`TEMP`/`TMP` after `env_clear` (no-op on
         Unix). DEAD_SERVER tests never connect, so they were unaffected.
      3. `clippy::result_large_err` on Windows only: `ConfigError::Parse`
         embeds a bulky `toml::de::Error` that tips the enum past 128 bytes
         on Windows. Boxed it — keeps every `Result<_, ConfigError>` small.
      4. `open_cache_storage` import was unused on Windows (its only user is
         a `cfg(unix)` test) → scoped the import into that test.
- [x] **No samod/networking bug**: once the env was right, all live `tcp://`
      sync tests passed on Windows. (Production uses `wss://` anyway.)
- [ ] Update br-9b5rxe75 with the above; close it (code is portable + proven
      by CI).

## Phase 2 — release artifact (done; exercised at next tag)

- [x] Added `windows_amd64` / `x86_64-pc-windows-msvc` (`windows-latest`) to
      the `release.yml` matrix with a per-entry `ext` (`tar.gz` vs `zip`).
- [x] Windows packs `braid.exe` into `braid-<version>-windows_amd64.zip`
      (pwsh `Compress-Archive`); its `.sha256` is written GNU-format
      (`<lowercase-hash>  <file>`, LF, no BOM) so the ubuntu combine job's
      `sha256sum -c` accepts it next to the unix lines.
- [x] Shared steps (version check, minisign install, sign) run via
      `shell: bash` (Git Bash on the Windows runner); minisign installed via
      `choco`. Signing + the pinned-key verify are identical across OSes.
- [x] Combine-job presence check + release-notes table include Windows; the
      `braid-release` skill's asset count updated 13 → 16.
- Note: the release build path only runs on a tag, so it's **validated at
  the next release**. The risky bits (zip + checksum format) are mirrored
  from the proven unix path and additionally exercised by the install.ps1
  CI smoke test below.

## Phase 3 — easy install

- [x] **PowerShell installer** (`install.ps1`): resolves the latest release
      (or `-Version`), downloads the zip, verifies SHA-256 (fetching the
      published `.sha256` or a passed `-Checksum`), extracts `braid.exe` to
      `%USERPROFILE%\.local\bin` (`-Dest` to override), and prints the PATH
      line. `-ArtifactUrl` accepts a local path for offline testing.
- [x] **CI smoke test** (windows-latest, in `ci.yml`): zip the just-built
      `braid.exe`, install it via `install.ps1` from that local artifact
      with checksum verification, assert `braid.exe --version`, and assert a
      bad checksum is rejected. Validates the installer without a release.
- [x] **Scoop manifest** (`packaging/scoop/braid.json`) + README —
      groundwork: autoupdate wired to the release URL + `.sha256`, but
      version/url/hash are placeholders until the first Windows release
      exists (and Scoop needs a bucket repo to be user-installable). See
      `packaging/scoop/README.md`.
- [x] Docs: README Windows section; release notes include the PowerShell
      one-liner.

Deferred: a release-time workflow that fills + commits the Scoop manifest
(beads' `update-package-manifests.yml` model) and a `cscheid/scoop-braid`
bucket; WinGet manifest; `aarch64-pc-windows-msvc`; Authenticode signing
(needs a paid cert — document the SmartScreen expectation instead).
