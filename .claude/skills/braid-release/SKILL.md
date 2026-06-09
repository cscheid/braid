---
name: braid-release
description: Cut a braid release (version bump + tag + verify). Use when the user asks to release, bump the version, or tag vX.Y.Z.
---

Releases are tag-driven: pushing `vX.Y.Z` triggers `.github/workflows/release.yml`
(5-target build → checksummed, minisign-signed archives → GitHub Release:
.tar.gz for the four unix targets, .zip for Windows; artifact naming is a
contract with `install.sh`, `install.ps1`, and
`crates/braid/tests/installer.rs`). The version in filenames has no `v`
prefix; the tag does. Signing needs the `MINISIGN_SECRET_KEY` repo secret
to match the public key pinned in `install.sh` (the sign step verifies
this and fails the release on mismatch).

Follow these steps in order. **Ask before every push** (CLAUDE.md git rule) —
the user asking for a release usually authorizes all of them, but say what
you're pushing.

1. **Pre-flight**: working tree clean; `main` rebased on `origin/main`; latest
   CI run on origin/main is green (`gh run list --repo cscheid/braid --limit 1`).
2. **Bump** `version` in the root `Cargo.toml` `[workspace.package]`.
3. **Version-matched docs**: update the `$comment` in
   `docs/schemas/strand.schema.json` (it names the braid version the schema
   ships with). Grep for the old version string to catch stragglers:
   `grep -rn "<old version>" --include="*.toml" --include="*.json" --include="*.md" . | grep -v target/`
4. **Refresh the lockfile**: `cargo build` (Cargo.lock picks up the new version).
5. **Gate**: `cargo xtask ci` (fmt + clippy -D warnings + full test suite).
6. **Commit** `release: vX.Y.Z` and push to `origin/main`; wait for CI green.
7. **Tag and push the tag**: `git tag vX.Y.Z && git push origin vX.Y.Z`.
8. **Watch the Release workflow** (`gh run watch`); then verify the release:
   `gh release view vX.Y.Z` must show 16 assets — 5 platform archives
   (`braid-X.Y.Z-{darwin,linux}_{arm64,amd64}.tar.gz` and
   `braid-X.Y.Z-windows_amd64.zip`),
   their 5 `.sha256` files, their 5 `.minisig` signatures, and
   `checksums.sha256`.
9. **Record it in the skein**: `BRAID_AUTHOR=claude braid comment` on a
   relevant strand, or a release note comment; `braid sync`.
10. Reinstall the local binary through the production path — it doubles as
    the final end-to-end check (checksum + signature against the pinned key):
    `curl -fsSL https://raw.githubusercontent.com/cscheid/braid/main/install.sh | bash`
    then confirm `braid --version` reports the new version.

Versioning judgment: braid is pre-1.0; bump the minor for feature batches
(new commands, MCP surface changes, schema-visible changes), the patch for
fixes. The document `schema_version` is independent — it only changes when
the automerge document shape breaks compatibility.
