# Installer for braid (port of beads_rust install.sh, simplified)

## Overview

Make braid installable on other machines via a `curl | bash` one-liner,
modeled on beads_rust's installer
([external-sources/beads_rust/install.sh](../../../../external-sources/beads_rust/install.sh))
but deliberately smaller. Their script is ~1700 lines; we target ~300-400
by cutting cosmetics and edge-case machinery while keeping the
security-relevant core.

This requires two artifacts we don't have yet:

1. `.github/workflows/release.yml` — builds and publishes versioned
   binaries + checksums on tag push (the installer has nothing to
   download today).
2. `install.sh` at the repo root, fetched via
   `curl -fsSL https://raw.githubusercontent.com/cscheid/braid/main/install.sh | bash`.

## How beads_rust's installation works (study notes)

Their flow has four pieces:

1. **`release.yml`**: on `v*` tag push, a 7-target matrix (linux
   gnu/musl × amd64/arm64, darwin amd64/arm64, windows) builds release
   binaries, names them `br-<ver>-<platform>.tar.gz`, generates per-file
   `.sha256` + a combined `checksums.sha256`, signs with minisign,
   attaches SLSA provenance + SBOM, creates the GitHub Release, and
   publishes to crates.io. A preflight job asserts the tag matches
   `Cargo.toml` version; runnable targets also assert `--version`
   output matches the tag.
2. **`install.sh`**: detects platform (os/arch/libc), resolves the
   latest version via GitHub API (with redirect-based fallback), downloads
   the artifact + checksum, verifies SHA256 (refusing unverified installs
   unless `--insecure-skip-checksum`), validates archive members against
   path traversal/symlinks, extracts, and atomically installs to
   `~/.local/bin`. Falls back to `cargo build` from a fresh clone if no
   release exists. Post-install: PATH advice, rc-file edits
   (`--easy-mode`), alias-conflict fixing, conflicting-binary detection,
   and installation of Claude/Codex skills.
3. **`tests/e2e_installer.rs`**: a Rust test file that drives the bash
   script — platform detection, checksum success/mismatch, idempotency,
   lock behavior, uninstall, `--help` coverage. Network-dependent tests
   self-skip offline.
4. **`packaging/`**: homebrew/scoop/aur manifests (kept in sync by a
   separate workflow).

### What we adopt

- **Release naming + checksum contract**: `braid-<ver>-<platform>.tar.gz`
  plus `<artifact>.sha256` and combined `checksums.sha256`. The installer
  and workflow agree on this contract; it's the load-bearing interface.
- **Checksum verification is mandatory by default** with an explicit
  `--insecure-skip-checksum` escape hatch. This is the single best
  security decision in their script.
- **Tag/Cargo.toml/`--version` triple-check** in the release workflow.
  Cheap, catches real mistakes.
- **Version resolution** via GitHub API with the redirect
  (`releases/latest` → `/tag/vX.Y.Z`) fallback. Two small functions.
- **Atomic install**: download to `$TMP`, `install -m 0755` to
  `dest.tmp.$$`, `mv -f` into place. Plus `trap cleanup EXIT` on a
  single mktemp dir.
- **Defaults**: `~/.local/bin` dest, `--dest DIR`, `--version vX.Y.Z`,
  `--uninstall`, `--quiet`, `--help`.
- **PATH handling**: warn with a copy-pasteable `export PATH=...` line
  when dest is not on PATH. (No rc-file editing — see below.)
- **`--from-source` fallback** via `git clone --depth 1` + `cargo build
  --release` with a TMP-scoped `CARGO_TARGET_DIR`. We *error with
  instructions* if cargo is missing instead of auto-installing rustup.
- **Rust-driven e2e tests for the installer** (their
  `e2e_installer.rs` pattern) — fits our existing `cargo test` CI, and
  `--artifact-url file://...` + `--checksum SHA` make the install path
  testable fully offline.
- **The `{ main "$@"; }` wrapper** at the bottom so a truncated
  `curl | bash` download can't execute half a script.

### What we deliberately skip (and why)

- **gum styling + gum auto-install** (~350 lines): the installer
  *installs another tool* (including adding apt/yum repos with sudo!)
  just to print pretty boxes. Plain ANSI colors, ~20 lines.
- **curl|bash self-re-exec bootstrap** (~70 lines): their fix for
  piped-stdin hazards (#250). The hazard exists because their script
  reads from stdin (interactive `gum confirm` / `read` prompts). Our
  installer is fully non-interactive, and the `{ main; }` wrapper covers
  truncation, so the whole bootstrap is unnecessary.
- **Lock file with stale-PID/age detection** (~50 lines): concurrent
  installs of the same tool on one machine are not a problem worth 50
  lines; atomic `mv` already prevents a torn binary.
- **rc-file editing** (`--easy-mode` PATH appends, `unalias br`
  injection): silently editing `~/.zshrc` is the most user-hostile part
  of their script. We print the export line and let the user decide.
- **Archive member validation** (tar/zip path-traversal + symlink
  checks, with a python3 fallback, ~110 lines): defends against the
  Zip-Slip family — archive members named `../../x` or `/etc/x`, or
  link-then-write symlink chains, escaping the extraction dir on
  extractors that honor them. Added by their April 2026 security audit
  (`beads_rust-dgo2`, commit `c6001e6e`). It matters for them because
  `--artifact-url`/`--checksum-url` mean the archive isn't necessarily
  their release, and checksums prove integrity, not authenticity (an
  attacker supplying both URL and checksum verifies cleanly). We skip
  it because the defense only covers the window between extraction and
  first execution: any hostile archive contains a hostile *binary* the
  installer is about to put on PATH, so the trust decision happens at
  URL choice, not extraction. braid's default flow only fetches our own
  GitHub release over TLS; `--artifact-url` is an explicit opt-in
  (mainly for offline tests). Revisit if we ever add `--system`/sudo
  installs — then a ~10-line bash check (reject `..`, leading `/`, and
  link entries in `tar -tvzf` output; tar.gz-only, no python) buys most
  of the protection.
- **Skills installation** into `~/.claude/skills` + `~/.codex/skills`
  (~150 lines incl. summary box): braid already has `braid agents-info`;
  if we ever ship a skill, it should be a separate opt-in command, not
  installer payload.
- **Windows artifacts**: our CI covers ubuntu + macos only. Windows
  users get `cargo install --git` for now; revisit on demand. This drops
  zip handling (unzip/bsdtar/python3 fallbacks) entirely.
- **minisign signing, SLSA attestations, SBOM, crates.io publish**:
  good practice at their maturity, premature at v0.1.0. Checksums first;
  note minisign as a future strand.
- **Homebrew/scoop/AUR packaging**: future work, separate strand.
- **wget fallback**: curl is preinstalled on macOS and effectively
  universal on dev Linux boxes. Requiring curl drops a duplicate code
  path from every download site. `die` with a clear message if absent.
- **Download resume (`--continue-at`), proxy env plumbing**: `curl
  --retry 3` handles transient failures; curl already honors proxy env
  vars natively without us forwarding them.

### Key simplification: musl-only Linux binaries

beads_rust ships gnu *and* musl Linux artifacts and carries ~40 lines of
libc detection (Alpine fast path, `/proc/self/maps` sniffing, `ldd`
output parsing, pipefail caveats). We instead ship **only statically
linked musl binaries for Linux** (`x86_64-unknown-linux-musl`,
`aarch64-unknown-linux-musl`). One artifact per arch runs on every
distro including Alpine; the installer's platform detection collapses to
os × arch. braid's deps (automerge, samod, tokio) are pure Rust, so musl
builds should be clean — Phase 0 verifies this before we commit.

Why beads_rust ships both (from their changelog/git history — it's scar
tissue, not principle): they started gnu-only, which broke on Ubuntu
22.04 (built against GLIBC_2.39, #36) → switched to musl-only → musl
broke because their custom SQLite stack (`uring-fs` via `fsqlite-wal`)
calls `libc::statx`, absent from musl libc (`bec2a3f`) → reverted to
gnu-only → gnu-only broke Alpine (gnu binaries reference
`libgcc_s`/`_Unwind_*` symbols musl's compat shim lacks, #284) → ship
both + 40 lines of libc detection to route between them. The blocker
(`statx` in `uring-fs`) is a dependency braid doesn't have, so musl-only
should work for us — but their history shows musl breakage can hide in
a transitive dep, which is exactly what the Phase 0 spike checks
(build *and* run the test suite under musl).

Artifact matrix (4 targets):

| platform string  | rust target                  | runner           |
|------------------|------------------------------|------------------|
| `linux_amd64`    | x86_64-unknown-linux-musl    | ubuntu-latest    |
| `linux_arm64`    | aarch64-unknown-linux-musl   | ubuntu-24.04-arm |
| `darwin_amd64`   | x86_64-apple-darwin          | macos-15         |
| `darwin_arm64`   | aarch64-apple-darwin         | macos-14         |

## Decisions to confirm

- [ ] musl-only Linux (vs gnu+musl like beads_rust) — recommended above
- [ ] No Windows artifacts in v1 — recommended above
- [ ] Skip minisign/SBOM/provenance in v1 — recommended above
- [ ] Installer filename/location: `install.sh` at repo root (matches
      the one-liner URL convention)

## Work Items

### Phase 0: feasibility spike

- [x] Verify `cargo check --target x86_64-unknown-linux-musl` — **found a
      blocker**: samod's `tungstenite` feature hardcodes `native-tls`,
      which on Linux means `openssl-sys`, which fails musl cross-builds.
      (On macOS native-tls uses Security.framework, so local builds never
      noticed.) Exactly the transitive-dep musl breakage beads_rust hit
      with `libc::statx`.
- [x] File a strand for this work (`BRAID_AUTHOR=claude`), linking this
      plan — `br-iju0n3gd`

**Phase 0 outcome.** musl-only stays viable, but needs a TLS-stack swap
first (new Phase 0.5). braid uses exactly one API from samod's
`tungstenite` feature — `repo.dial_websocket()` — which is a thin
convenience wrapper over the *public* `Repo::dial(backoff, Arc<dyn
Dialer>)` + `Transport::new()` API. We implement our own websocket
dialer on a direct `tokio-tungstenite` dep with
`rustls-tls-webpki-roots` (embedded Mozilla roots — right for a static
binary that must work in containers without `ca-certificates`), and
drop samod's `tungstenite` feature. Test-side websocket accepts use
`tokio_tungstenite::accept_async` + `acceptor.accept(Transport::new(..))`,
reusing the same message↔bytes conversion as the dialer.

Caveat: rustls' crypto provider (aws-lc-rs or ring) contains C that
needs a musl cross-compiler, absent on this Mac — so local verification
stops at `cargo check` of pure-Rust crates; the authoritative check is
a CI musl job, which we add permanently as release-target regression
protection.

### Phase 0.5: rustls websocket dialer (musl prerequisite)

Tracked as its own strand (discovered-from `br-iju0n3gd`).

- [x] TDD baseline: extend `crates/braid/tests/sync.rs` with a `ws://`
      loopback server variant (websocket accept loop) and a sync
      round-trip e2e test — green against the *current* native-tls
      implementation, establishing the behavior contract before the swap
- [x] Implement `WsDialer` (new module `crates/braid/src/ws.rs`):
      `samod::Dialer` impl over `tokio_tungstenite::connect_async`,
      message↔bytes conversion (Binary passes, Ping/Pong/Close filtered,
      Text is an error) shared between dial and test-accept paths
- [x] `sync.rs`: replace `repo.dial_websocket(url, backoff)` with
      `repo.dial(backoff, Arc::new(WsDialer::new(url)))`
- [x] Cargo.toml: drop `tungstenite` from samod features; direct
      `tokio-tungstenite` dep with `connect`/`handshake`/
      `rustls-tls-webpki-roots`; rustls pinned to the **ring** provider
      (default-features off) — the default aws-lc-rs needs cmake on some
      hosts and is heavier to cross-compile to musl
- [x] Verify `cargo tree --target x86_64-unknown-linux-musl -i
      openssl-sys` is empty (also `native-tls`: gone); full local test
      suite green (19 suites), clippy clean. Local musl `cargo check`
      now stops only at ring's C code needing a musl cross-gcc this Mac
      doesn't have — a local-toolchain gap, not a dependency problem;
      CI is the gate.
- [x] CI: add a musl job (ubuntu + musl-tools, build + `cargo test
      --target x86_64-unknown-linux-musl`) — the authoritative musl gate
- [x] wss:// verification: dogfooded — the new dialer syncs this repo's
      own skein against `wss://sync.automerge.org` (real CA chain, real
      server). An automated wss e2e (rcgen self-signed + injectable TLS
      config) is deferred; ws:// e2e covers the dialer logic and rustls
      itself is upstream-tested.

### Phase 1: test specifications (TDD — before install.sh exists)

- [x] Add `crates/braid/tests/installer.rs` (26 tests) modeled on
      beads_rust's, offline-first via `--artifact-url file://...` +
      `--checksum`, `uname` faked through a PATH shim:
  - [x] platform detection: maps `uname` combos to the 4 platform strings,
        dies on unsupported OS/arch pointing at `cargo install`
  - [x] `--help` lists every supported flag; unknown flags are
        **rejected** (a typo silently ignored is how a `--checksun`
        install ends up unverified)
  - [x] successful install from a local artifact: binary lands in
        `--dest`, is executable, dest dir is created if missing; stdout
        stays empty (progress is stderr-only)
  - [x] checksum mismatch → nonzero exit, nothing installed, no partial
        files left in dest; malformed checksum rejected
  - [x] missing checksum + no `--insecure-skip-checksum` → refuses,
        naming the escape hatch
  - [x] missing checksum + `--insecure-skip-checksum` → proceeds with
        loud UNVERIFIED warning
  - [x] `.sha256` sidecar next to the artifact found automatically
  - [x] idempotency: running twice succeeds, binary still works
  - [x] `--uninstall` removes the binary; uninstall-when-absent still
        exits 0 with a notice
  - [x] PATH warning printed when dest not on PATH; absent when it is
  - [x] `--quiet` suppresses progress entirely (empty output on clean
        install) but never errors
  - [x] dest precedence: `--dest` > `BRAID_INSTALL_DIR` > `~/.local/bin`
  - [x] archive without a `braid` member fails cleanly
  - [x] shellcheck clean at `--severity=style` (skips if unavailable)
- [x] Network-dependent test: version resolution against the real GitHub
      API — written as `#[ignore]`, to be run in Phase 4 once a release
      exists

### Phase 2: install.sh

- [x] Write `install.sh` (~370 lines): config/flags, plain-ANSI
      logging (colors only when stderr is a tty), platform detection
      (os × arch only), version resolution (API + redirect fallback),
      `download_file` (curl-only, retry, `.part` + atomic mv), SHA256
      verify (sha256sum/shasum), extract tar.gz, atomic install,
      PATH advice, `--from-source`, `--uninstall`, `--print-platform`,
      `trap` cleanup, `{ main "$@"; }` wrapper
- [x] All Phase 1 tests green (26/26; TDD red phase confirmed first)
- [x] shellcheck clean at `--severity=style` (installed locally via brew
      so the conditional test exercises during development, not just CI)
- [x] Smoke test with the real binary: tar.gz of `target/debug/braid`,
      installed via the sidecar-checksum path, runs and reports its
      version

Decision recorded along the way: failure to resolve the latest version
**dies** suggesting `--version`/`--from-source` rather than silently
falling back to a multi-minute source build (explicit > implicit;
diverges from beads_rust).

### Phase 3: release workflow

- [x] `.github/workflows/release.yml`: preflight tag-vs-Cargo.toml
      check → 4-target build matrix → binary `--version`-vs-tag check
      (every target executes on its runner: musl is static, darwin
      x86_64 runs under Rosetta) → archive as
      `braid-<ver>-<platform>.tar.gz` → per-artifact `.sha256` +
      combined, verified `checksums.sha256` → GitHub Release via `gh
      release create` with generated notes. Full `cargo test` in the
      release matrix was dropped as redundant: CI (incl. the musl job)
      already gates every commit on main; the release jobs verify the
      built artifact itself.
- [x] Workflow-lint: actionlint clean (installed via brew; it also
      shellchecks the embedded run blocks)
- [x] `--locked` release build verified locally; version-string parsing
      (`braid 0.1.0` → `0.1.0`) confirmed against the real binary
- [x] `[profile.release] strip = true` — shipped binary 8.5M → 7.0M
- [x] Tag `v0.1.0`, confirm artifacts + checksums appear — release
      published with all 9 assets (4 platforms × archive+sha256, plus
      combined checksums.sha256). Follow-up found in the run logs:
      upload/download-artifact@v4 are Node 20 (forced to Node 24 on
      2026-06-16); bumped to @v7/@v8.

### Phase 4: end-to-end validation + docs

- [x] Run the real one-liner against the live v0.1.0 release
      (darwin_arm64): resolution → download → published-checksum
      verification → install → `braid 0.1.0`. (linux_amd64 covered by
      the release job executing the static binary; no local container
      runtime available for a distro-matrix check — revisit if Linux
      install reports problems.)
- [x] Run the `#[ignore]`d network e2e test against the real release —
      passes
- [x] README: Installation section (one-liner, flags, cargo-install
      alternative, uninstall)
- [x] File future-work strands: minisign signing (`br-dgvi0nme`),
      homebrew tap (`br-2vr7pewh`), Windows support (`br-jrt1n7pl`),
      `braid self-update` (`br-e8oyaptw`)
- [x] Close the Phase 0.5 strand `br-f3b18xoa`

## Details

### Installer interface (target)

```
curl -fsSL https://raw.githubusercontent.com/cscheid/braid/main/install.sh | bash
curl -fsSL .../install.sh | bash -s -- [OPTIONS]

  --version vX.Y.Z          install specific version (default: latest)
  --dest DIR                install dir (default: ~/.local/bin; env BRAID_INSTALL_DIR)
  --artifact-url URL        custom artifact URL (also enables offline tests)
  --checksum SHA            expected SHA256
  --insecure-skip-checksum  allow unverified install (loud warning)
  --from-source             git clone + cargo build --release
  --uninstall               remove installed binary
  --quiet                   errors only
  --help
```

No interactive prompts anywhere — this is what lets us drop the
self-re-exec bootstrap and makes the script safe under `curl | bash` and
in CI.

### Release/installer artifact contract

```
braid-<version>-<platform>.tar.gz          # contains a single file: braid
braid-<version>-<platform>.tar.gz.sha256   # "<sha256>  <filename>"
checksums.sha256                           # all of the above, concatenated
```

`<version>` has no `v` prefix in filenames; the release tag does
(`v0.1.0` → `braid-0.1.0-darwin_arm64.tar.gz`), matching beads_rust's
`release_download_tag`/`release_asset_version` convention.

### Test harness notes

beads_rust's `run_installer` helper spawns `bash install.sh` with a
TempDir-scoped `--dest` and env overrides; `shell_function_section`
slices a single function out of the script and `bash -c`-evaluates it to
unit-test platform detection without running main. Both patterns port
directly. We gate platform-detection unit tests on faked `uname` via a
PATH-shimmed stub directory rather than parsing the script, where
possible.
