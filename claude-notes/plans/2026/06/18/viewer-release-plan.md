# braid-viewer Release Bundles — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every `v*` release tag publishes unsigned braid-viewer GUI bundles (macOS `.dmg`, Windows NSIS, Linux `.AppImage`+`.deb`) attached to the same GitHub Release as the CLI binaries.

**Architecture:** Add a `viewer` build job to `.github/workflows/release.yml`, parallel to the existing `build` (CLI) job; both feed the existing `release` job, which stays the single release-creation point. The viewer job builds bundles per target through an extended `cargo xtask viewer-build` that forwards `--target`/`--bundles` to `cargo tauri build`. A version-drift fix makes Tauri inherit the workspace version.

**Tech Stack:** GitHub Actions, Tauri v2 (`cargo tauri build`), Rust xtask (std-only), Node/npm (Vite UI), bash packaging.

**Design spec:** `claude-notes/plans/2026/06/18/viewer-release-design.md`
**Signing follow-up:** braid strand `bd-ija7ely5` (out of scope here).

## Global Constraints

- **Unsigned bundles only** — no code signing, no minisign on the viewer path. Integrity-only `SHA256SUMS-viewer`.
- **Bundle subset per OS:** macOS `dmg`, Windows `nsis`, Linux `deb,appimage`. Do not ship `.app`/`.msi`/`.rpm`.
- **Both Linux arches on `ubuntu-22.04`** (x86) / **`ubuntu-22.04-arm`** (arm) — shared glibc floor; never `24.04`.
- **Pinned Tauri default bundle filenames** (`{productName}_{version}_{arch}.{ext}`, `productName=braid-viewer`): `braid-viewer_<v>_aarch64.dmg`, `braid-viewer_<v>_x64.dmg`, `braid-viewer_<v>_x64-setup.exe`, `braid-viewer_<v>_amd64.AppImage`, `braid-viewer_<v>_amd64.deb`, `braid-viewer_<v>_aarch64.AppImage`, `braid-viewer_<v>_arm64.deb`.
- **Single build path:** CI builds via `cargo xtask viewer-build -- …`, never a direct `cargo tauri build`.
- **No Scoop/WinGet in user-facing release notes** (no viewer manifest exists).
- xtask stays **std-only** (no new crate dependencies).
- Per-task commits; attribute braid changes with `BRAID_AUTHOR=claude`.

---

### Task 1: Version-drift fix + guard test

`crates/braid-viewer/tauri.conf.json` hardcodes `"version": "0.4.0"`; the crate uses `version.workspace`. Remove the hardcoded version so Tauri inherits the workspace version (bundle filenames then carry the release tag). Guard with an xtask test so it can't return.

**Files:**
- Modify: `crates/braid-viewer/tauri.conf.json` (remove the `version` line)
- Test: `crates/xtask/src/main.rs` (new test in the existing `mod tests`)

**Interfaces:**
- Consumes: nothing.
- Produces: nothing new in code; establishes the invariant "tauri.conf.json has no `version` key".

- [ ] **Step 1: Write the failing test**

In `crates/xtask/src/main.rs`, add to `mod tests`:

```rust
    #[test]
    fn tauri_conf_has_no_hardcoded_version() {
        // braid-viewer's tauri.conf.json must omit `version` so Tauri
        // inherits the workspace crate version; a hardcoded value drifts
        // from the release tag. See
        // claude-notes/plans/2026/06/18/viewer-release-design.md.
        let conf = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../braid-viewer/tauri.conf.json");
        let content = std::fs::read_to_string(&conf)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", conf.display()));
        assert!(
            !content.contains("\"version\""),
            "tauri.conf.json must not hardcode a version (inherit the \
             workspace crate version instead); found in {}",
            conf.display()
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p xtask tauri_conf_has_no_hardcoded_version`
Expected: FAIL — assertion fires (the `"version": "0.4.0"` key is still present).

- [ ] **Step 3: Remove the hardcoded version**

In `crates/braid-viewer/tauri.conf.json`, delete the line:

```json
  "version": "0.4.0",
```

(Leave `productName` above and `identifier` below intact.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p xtask tauri_conf_has_no_hardcoded_version`
Expected: PASS.

- [ ] **Step 5: Confirm Tauri still resolves a version (sanity, optional locally)**

If `cargo tauri` is installed locally: `cargo xtask viewer-build` and confirm the produced bundle filenames carry the workspace version (e.g. `braid-viewer_0.5.0_…`). If Tauri tooling is not available locally, this is verified in Task 6's prerelease run instead — do not block.

- [ ] **Step 6: Commit**

```bash
git add crates/braid-viewer/tauri.conf.json crates/xtask/src/main.rs
git commit -m "fix(viewer): inherit workspace version in tauri.conf

Hardcoded 0.4.0 drifted from the release tag; Tauri reads the crate
version when tauri.conf omits it. An xtask test guards re-introduction."
```

---

### Task 2: Extend `cargo xtask viewer-build` to forward args

`viewer_tauri` runs a bare `cargo tauri build` with no passthrough, so CI can't pass `--target`/`--bundles`. Extract a pure `tauri_argv` (mirrors the existing testable `viewer_preflight` pattern), test it, then wire it through `viewer_tauri` and `main`.

**Files:**
- Modify: `crates/xtask/src/main.rs` (add `tauri_argv`; change `viewer_tauri` signature; update `main` dispatch and `USAGE`)
- Test: `crates/xtask/src/main.rs` (`mod tests`)

**Interfaces:**
- Produces: `fn tauri_argv(subcommand: &str, extra: &[String]) -> Vec<String>` and `fn viewer_tauri(subcommand: &str, extra: &[String]) -> i32`.
- Consumes (CI, Task 3): `cargo xtask viewer-build -- --target <triple> --bundles <kinds>`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/xtask/src/main.rs`:

```rust
    use super::tauri_argv;

    fn as_strs(v: &[String]) -> Vec<&str> {
        v.iter().map(String::as_str).collect()
    }

    #[test]
    fn tauri_argv_bare_subcommand() {
        assert_eq!(as_strs(&tauri_argv("build", &[])), ["tauri", "build"]);
    }

    #[test]
    fn tauri_argv_appends_passthrough() {
        let extra = vec!["--target".to_string(), "aarch64-apple-darwin".to_string()];
        assert_eq!(
            as_strs(&tauri_argv("build", &extra)),
            ["tauri", "build", "--target", "aarch64-apple-darwin"]
        );
    }

    #[test]
    fn tauri_argv_strips_leading_separator() {
        // `cargo xtask viewer-build -- --bundles dmg` delivers a leading
        // `--` in the passthrough; it must not reach `cargo tauri`.
        let extra = vec!["--".to_string(), "--bundles".to_string(), "dmg".to_string()];
        assert_eq!(
            as_strs(&tauri_argv("build", &extra)),
            ["tauri", "build", "--bundles", "dmg"]
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p xtask tauri_argv`
Expected: FAIL to compile — `tauri_argv` not defined.

- [ ] **Step 3: Add `tauri_argv` and rewire `viewer_tauri`**

In `crates/xtask/src/main.rs`, add above `viewer_tauri`:

```rust
/// Build the argv for `cargo tauri <subcommand>`, appending caller
/// passthrough args. A single leading `--` separator (as inserted by
/// `cargo xtask viewer-build -- …`) is dropped so it never reaches
/// `cargo tauri`.
fn tauri_argv(subcommand: &str, extra: &[String]) -> Vec<String> {
    let mut argv = vec!["tauri".to_string(), subcommand.to_string()];
    let extra = match extra.split_first() {
        Some((first, rest)) if first == "--" => rest,
        _ => extra,
    };
    argv.extend(extra.iter().cloned());
    argv
}
```

Change the `viewer_tauri` signature and its `cargo tauri` invocation:

```rust
fn viewer_tauri(subcommand: &str, extra: &[String]) -> i32 {
```

Replace the run block (currently `Command::new("cargo").args(["tauri", subcommand])…`) with:

```rust
    let argv = tauri_argv(subcommand, extra);
    eprintln!("xtask: cargo {} in {}", argv.join(" "), viewer_dir.display());
    match Command::new("cargo").args(&argv).current_dir(&viewer_dir).status() {
        Ok(st) if st.success() => 0,
        Ok(st) => {
            eprintln!("xtask: FAILED ({st}): cargo {}", argv.join(" "));
            1
        }
        Err(e) => {
            eprintln!("xtask: cannot run cargo {}: {e}", argv.join(" "));
            eprintln!(
                "xtask: is tauri-cli installed? \
                 (cargo install tauri-cli --version '^2')"
            );
            1
        }
    }
```

Update the two `main` dispatch arms:

```rust
        Some("viewer-dev") => viewer_tauri("dev", &args[1..]),
        Some("viewer-build") => viewer_tauri("build", &args[1..]),
```

Update the `USAGE` line for `viewer-build` to note passthrough:

```rust
  viewer-build     build the braid-viewer Tauri app bundle (`cargo tauri build`);
                   args after `--` pass through, e.g.
                   `cargo xtask viewer-build -- --target <triple> --bundles dmg`
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p xtask`
Expected: PASS (new `tauri_argv` tests + existing `preflight`/`tauri_conf` tests).

- [ ] **Step 5: Confirm clippy/fmt clean**

Run: `cargo clippy -p xtask --all-targets -- -D warnings && cargo fmt -p xtask -- --check`
Expected: no warnings, no diff.

- [ ] **Step 6: Commit**

```bash
git add crates/xtask/src/main.rs
git commit -m "feat(xtask): forward passthrough args from viewer-build to cargo tauri

CI needs per-target, bundle-restricted builds; viewer-build now forwards
args after \`--\` to \`cargo tauri build\`, keeping one build path for local
and CI."
```

---

### Task 3: Add the `viewer` build job to `release.yml`

Add a matrix job that builds the bundles per target and uploads them as workflow artifacts. CI YAML is not unit-testable; correctness is verified by Task 6's prerelease run.

**Files:**
- Modify: `.github/workflows/release.yml` (new `viewer` job after the `build` job, before `release`)

**Interfaces:**
- Consumes: `needs.preflight.outputs.version`; `cargo xtask viewer-build -- …` (Task 2).
- Produces: workflow artifacts `braid-viewer-<platform>` containing the staged bundle files (consumed by Task 4's `release` job).

- [ ] **Step 1: Insert the `viewer` job**

In `.github/workflows/release.yml`, add this job between the `build` job and the `release` job (sibling indentation under `jobs:`):

```yaml
  viewer:
    name: Viewer bundle (${{ matrix.platform }})
    needs: preflight
    runs-on: ${{ matrix.os }}
    timeout-minutes: 45
    strategy:
      fail-fast: false
      matrix:
        include:
          - platform: darwin_arm64
            target: aarch64-apple-darwin
            os: macos-15
            bundles: dmg
          - platform: darwin_amd64
            target: x86_64-apple-darwin
            os: macos-15
            bundles: dmg
          - platform: windows_amd64
            target: x86_64-pc-windows-msvc
            os: windows-latest
            bundles: nsis
          - platform: linux_amd64
            target: x86_64-unknown-linux-gnu
            os: ubuntu-22.04
            bundles: deb,appimage
          - platform: linux_arm64
            target: aarch64-unknown-linux-gnu
            os: ubuntu-22.04-arm
            bundles: deb,appimage
    steps:
      - uses: actions/checkout@v5
        with:
          ref: ${{ github.event.inputs.tag || github.ref }}
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: npm
          cache-dependency-path: ui/package-lock.json
      - name: Install Linux WebKit dependencies
        if: runner.os == 'Linux'
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libwebkit2gtk-4.1-dev libgtk-3-dev \
            libayatana-appindicator3-dev librsvg2-dev \
            libsoup-3.0-dev build-essential
      - uses: Swatinem/rust-cache@v2
        with:
          key: viewer-release-${{ matrix.target }}
      # cache-bin (rust-cache default) persists this across runs.
      - name: Install tauri-cli
        run: cargo install tauri-cli --version '^2' --locked
      - name: Install UI deps
        run: npm ci
        working-directory: ui
      - name: Build viewer bundle
        run: cargo xtask viewer-build -- --target ${{ matrix.target }} --bundles ${{ matrix.bundles }}
      - name: Collect bundles and verify version
        shell: bash
        run: |
          VERSION="${{ needs.preflight.outputs.version }}"
          BUNDLE_DIR="target/${{ matrix.target }}/release/bundle"
          mkdir -p viewer-dist
          shopt -s nullglob
          files=( "$BUNDLE_DIR"/dmg/*.dmg \
                  "$BUNDLE_DIR"/nsis/*-setup.exe \
                  "$BUNDLE_DIR"/deb/*.deb \
                  "$BUNDLE_DIR"/appimage/*.AppImage )
          if [ ${#files[@]} -eq 0 ]; then
            echo "::error::no bundles found under $BUNDLE_DIR"; exit 1
          fi
          for f in "${files[@]}"; do
            base="$(basename "$f")"
            case "$base" in
              *"$VERSION"*) ;;
              *) echo "::error::bundle '$base' missing version $VERSION (drift?)"; exit 1 ;;
            esac
            cp "$f" "viewer-dist/$base"
          done
          echo "staged:"; ls -1 viewer-dist
      - uses: actions/upload-artifact@v7
        with:
          name: braid-viewer-${{ matrix.platform }}
          path: viewer-dist/*
          retention-days: 7
          if-no-files-found: error
```

- [ ] **Step 2: Validate the workflow YAML parses**

Run (PowerShell, Windows): `Get-Content .github/workflows/release.yml -Raw | python -c "import sys,yaml; yaml.safe_load(sys.stdin.read()); print('ok')"`
(or any YAML linter available). Expected: `ok` / no parse error.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci(release): build braid-viewer bundles per target

Adds a viewer matrix job (mac arm64/x86_64 dmg, win nsis, linux
x86_64/arm64 appimage+deb on the 22.04 glibc floor) that stages bundles
and verifies the filename version, uploading them as workflow artifacts."
```

---

### Task 4: Wire viewer bundles into the `release` job

Make the `release` job depend on `viewer`, validate the bundles are present, checksum them, and attach them to the GitHub Release.

**Files:**
- Modify: `.github/workflows/release.yml` (the `release` job: `needs`, the validate step, the asset list)

**Interfaces:**
- Consumes: `braid-viewer-<platform>` artifacts (Task 3) — flattened into `artifacts/` by the existing `download-artifact` with `merge-multiple: true`.
- Produces: `SHA256SUMS-viewer` and viewer bundles on the release.

- [ ] **Step 1: Add `viewer` to `needs`**

Change:

```yaml
    needs: [preflight, build]
```

to:

```yaml
    needs: [preflight, build, viewer]
```

- [ ] **Step 2: Validate viewer bundles present + checksum them**

In the "Validate all platforms present, combine and verify checksums" step, after the existing CLI `MISSING` loop and before `cat -- *.sha256 …`, add a viewer validation + checksum block:

```bash
          # braid-viewer GUI bundles (unsigned; integrity-only checksums).
          # Tauri default names — arch tokens differ per format by design.
          for f in \
            "braid-viewer_${VERSION}_aarch64.dmg" \
            "braid-viewer_${VERSION}_x64.dmg" \
            "braid-viewer_${VERSION}_x64-setup.exe" \
            "braid-viewer_${VERSION}_amd64.AppImage" \
            "braid-viewer_${VERSION}_amd64.deb" \
            "braid-viewer_${VERSION}_aarch64.AppImage" \
            "braid-viewer_${VERSION}_arm64.deb"; do
            [ -f "$f" ] || MISSING+=("$f")
          done
```

Then, after the existing `MISSING` check block exits non-zero on gaps, add (still inside the same step, after `sha256sum -c checksums.sha256`):

```bash
          sha256sum braid-viewer_* > SHA256SUMS-viewer
          sha256sum -c SHA256SUMS-viewer
          cat SHA256SUMS-viewer
```

- [ ] **Step 3: Attach viewer assets to the release**

In the "Create release" step, extend the `gh release create` asset list (the trailing arguments) with:

```bash
            artifacts/braid-viewer_* artifacts/SHA256SUMS-viewer \
```

(Add this line alongside the existing `artifacts/braid-*.tar.gz …` arguments, before the closing of the command.)

- [ ] **Step 4: Validate the workflow YAML parses**

Run the same YAML parse check as Task 3 Step 2. Expected: `ok`.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci(release): attach braid-viewer bundles to the release

release now depends on the viewer job, validates the pinned bundle
filenames, writes SHA256SUMS-viewer (integrity only — unsigned path),
and uploads the bundles alongside the CLI artifacts."
```

---

### Task 5: Release notes + README docs

Add the viewer downloads table and unsigned-install guidance to the generated release notes, and a download pointer in the README. No `agents-info.md` / `docs/mcp.md` change (no new braid subcommand or MCP tool), so `docs_drift.rs` is unaffected.

**Files:**
- Modify: `.github/workflows/release.yml` ("Generate release notes" step heredoc)
- Modify: `README.md` (viewer/desktop section)

**Interfaces:** none.

- [ ] **Step 1: Add viewer section to the notes heredoc**

In the "Generate release notes" step, inside the `{ … } > notes.md` heredoc, after the CLI platform table and before `echo "## Changes"`, add:

```bash
            echo "## Desktop viewer (braid-viewer)"
            echo
            echo "Unsigned GUI bundles — first launch:"
            echo "- macOS: right-click the app -> Open (Gatekeeper), or \`xattr -d com.apple.quarantine <app>\`."
            echo "- Windows: the installer is unsigned -> SmartScreen -> More info -> Run anyway."
            echo
            echo "| Platform | File |"
            echo "|---|---|"
            echo "| macOS Apple Silicon | \`braid-viewer_${VERSION}_aarch64.dmg\` |"
            echo "| macOS Intel | \`braid-viewer_${VERSION}_x64.dmg\` |"
            echo "| Windows x86_64 | \`braid-viewer_${VERSION}_x64-setup.exe\` |"
            echo "| Linux x86_64 (AppImage) | \`braid-viewer_${VERSION}_amd64.AppImage\` |"
            echo "| Linux x86_64 (deb) | \`braid-viewer_${VERSION}_amd64.deb\` |"
            echo "| Linux ARM64 (AppImage) | \`braid-viewer_${VERSION}_aarch64.AppImage\` |"
            echo "| Linux ARM64 (deb) | \`braid-viewer_${VERSION}_arm64.deb\` |"
            echo "Viewer checksums: \`sha256sum -c SHA256SUMS-viewer --ignore-missing\`"
            echo
```

(No Scoop/WinGet mention — per Global Constraints.)

- [ ] **Step 2: Add a README download pointer**

In `README.md`, in the existing viewer/desktop section (added in #13), add a short note that desktop bundles ship on each GitHub Release, are unsigned, and link to the Releases page. Match the surrounding README prose style. Example line:

```markdown
Desktop bundles (macOS `.dmg`, Windows installer, Linux `.AppImage`/`.deb`)
are attached to each [release](../../releases). They are currently unsigned —
on macOS, right-click → Open the first time; on Windows, choose "More info →
Run anyway" at the SmartScreen prompt.
```

- [ ] **Step 3: Validate YAML still parses**

Run the YAML parse check (Task 3 Step 2). Expected: `ok`.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release.yml README.md
git commit -m "docs(release): document viewer bundles and unsigned-install steps"
```

---

### Task 6: Prerelease validation (integration test)

CI workflow logic is verified end-to-end on a throwaway prerelease tag, not a real release. This is the integration test for Tasks 3–5 and confirms the Task 1 version inheritance and the pinned filenames.

**Files:** none (operational).

**Interfaces:** none.

- [ ] **Step 1: Push the branch and open a draft PR** (see Dev workflow). Wait for the `viewer.yml` smoke check to pass (compile check is unaffected by these changes).

- [ ] **Step 2: Run a prerelease build via `workflow_dispatch`**

Ask Carlos/Chris before pushing any tag (per repo + CLAUDE.local git rules). With approval, create a throwaway prerelease tag whose value matches the workspace `Cargo.toml` version with a prerelease suffix is **not** possible (preflight requires `vX.Y.Z == Cargo.toml`). So instead: trigger `release.yml` via `workflow_dispatch` against an existing tag, OR temporarily bump the workspace version on the branch and tag it. Preferred: coordinate with Carlos to run `workflow_dispatch` with `tag` = the next planned version after a real version bump, on the branch, and inspect artifacts without publishing.

Practical check that needs no publish: download the per-platform `braid-viewer-<platform>` workflow artifacts from the dispatched run and confirm:
- Every pinned filename in Global Constraints is present, version token = the dispatched version.
- If any filename differs (Tauri changed a default arch token), update the pinned list in Task 3's collect step, Task 4's validate block, and Task 5's notes table to match the actual names, then re-run.

- [ ] **Step 3: Smoke-launch a bundle**

On at least one platform, install/run a bundle:
- Linux: `sudo apt install ./braid-viewer_<v>_amd64.deb` on a clean container; launch; confirm WebKitGTK deps resolve and the window opens (not a blank `localhost:5173` — that would mean `custom-protocol` was missing).
- Or macOS: open the `.dmg`, drag to Applications, right-click → Open, confirm it launches.

- [ ] **Step 4: Re-run roborev security review on the implementation commits**

The implementation touches secrets/token surface (`GH_TOKEN`, `permissions: contents: write`, shell interpolation of version/filenames). Run `roborev review --type security` and `roborev review` (default) on the branch HEAD; triage findings via `/superpowers:receiving-code-review`.

- [ ] **Step 5: Final CI gate**

Run: `cargo xtask ci`
Expected: fmt, clippy, build, test, UI tests all pass (includes the Task 1 + Task 2 xtask tests).

---

## Dev workflow

- **Branch:** `feat/viewer-release` (already created; design commits live there).
- **Remotes:** `origin` = `cderv/braid` (fork), `upstream` = `cscheid/braid` (Carlos's main). Chris has push to `upstream`.
- **Push:** after committing, **stop and let Chris review the diff locally** before pushing. Do not push to a remote without asking Carlos (per repo CLAUDE.md git rule). Then push the branch and open a PR — never commit/merge `main` directly (CLAUDE.local).
- **Tags:** never push a `v*` tag without explicit approval — it triggers a real release.
- **Strand:** mark the implementing strand `in_progress` / `assignee claude` at start and close it with an outcome comment when the PR lands. Signing stays tracked separately in `bd-ija7ely5`.
- **Commits:** one per task as above; run `cargo xtask ci` before requesting a push.

## Self-review notes

- **Spec coverage:** workflow-shape (inline) → Tasks 3+4; matrix/floor → Task 3; build path (xtask forwarding) → Task 2; version drift → Task 1; naming pinned → Tasks 3/4/5; checksums → Task 4; docs/threat-model/unsigned note → Task 5; Linux deps + prerelease validation → Task 6; signing → out of scope (`bd-ija7ely5`).
- **Risk:** exact Tauri default filenames are assumed; Task 6 Step 2 is the explicit checkpoint to correct them in one place set if Tauri's tokens differ.
