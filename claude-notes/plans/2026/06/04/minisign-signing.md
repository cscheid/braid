# Sign release artifacts with minisign; verify in install.sh

Strand: br-dgvi0nme. Plan drafted 2026-06-04.

## Overview

Checksums prove integrity, not authenticity: the artifact and its
`.sha256` come from the same GitHub release, so whoever can replace one
can replace both. We adopt minisign (Ed25519, signify-compatible — the
tool Zig signs all releases with) to sign each release archive in CI and
verify in `install.sh` against a public key pinned in the script itself
— a different trust path (main branch) than the release assets.

### Decisions (settled with Carlos, 2026-06-04)

- **Strict by default.** minisign is packaged everywhere that matters
  (Homebrew, MacPorts, Debian/Ubuntu, Fedora/EPEL, Alpine, Arch, Nix,
  FreeBSD; repology shows no mainstream gaps), so `install.sh` *refuses*
  to install without signature verification unless
  `--insecure-skip-signature` is passed (name follows the existing
  `--insecure-skip-checksum` convention; Carlos sketched
  `--unsafe-no-signature-check`, same semantics). The refusal message
  gives per-platform install one-liners for minisign. Rationale: users
  re-run install.sh over the project's lifetime (no self-update by
  design), so the verifier being present is worth insisting on.
  This is *stricter than beads_rust* (whose installer never verifies
  signatures — signing exists only in their release.yml) and as strict
  as Zig's community-mirror protocol, where tooling MUST verify.
- **Trusted comment = archive filename** (`-t "$ARCHIVE"`), Zig-style:
  the comment is part of the signed payload, and install.sh compares the
  verified comment against the archive it asked for. This prevents
  replaying a validly-signed *different* artifact (e.g. an older
  version) under a new name. Without the installer-side comparison the
  comment is decorative — the check is part of the contract.
- **Key handling:** keypair generated locally with `minisign -G -W`
  (unencrypted — CI cannot answer a password prompt, and a password
  stored next to the key adds nothing). Secret key → `MINISIGN_SECRET_KEY`
  GitHub Actions secret; Carlos backs up secret+public key to his
  Bitwarden, then deletes the local file. Public key is pinned in
  install.sh and README. The key file never enters the conversation
  transcript or the repo. Residual risk (accepted): workflow-write
  access to the repo can sign; this closes the release-asset-only
  compromise gap.
- **Checksum verification stays** unchanged and mandatory — it is
  redundant for security once signatures verify, but cheap, and remains
  the integrity check under `--insecure-skip-signature`.

### Artifact contract additions (release.yml header ↔ install.sh ↔ installer.rs)

```
braid-<version>-<platform>.tar.gz.minisig    minisign signature of the archive
                                             trusted comment: the archive filename
```

Existing v0.1.0/v0.2.0 releases are unsigned; once strict install.sh
lands on main, default installs of those versions fail. Mitigation: cut
v0.2.1 promptly after landing (braid-release skill).

## Phase 0 — keypair + secret (with Carlos, before code)

- [x] `minisign -G -W` into a directory outside the repo; never read the
      secret key file in-session (`~/braid-minisign-key/`)
- [x] `gh secret set MINISIGN_SECRET_KEY < braid-release.key` (Carlos
      approved; touches GitHub repo settings)
- [ ] Hand Carlos the file paths for Bitwarden backup (secret + public
      key together); he deletes the local copies after
      (handed off 2026-06-04; pending Carlos)
- [x] Record the public key string for use in Phases 2–4:
      `RWSbWhSzVkkTRO4nFMzL/KyRs9oicbgy/2KPRK+o9hxznRYx9ZkHwwlN`
      (key ID 44134956B3145A9B)

## Phase 1 — tests first (TDD) — done 2026-06-04

All items below landed; red phase showed 16 failures all of the
"unknown option" kind, green phase passes 32/32 (+1 ignored network
test). One deviation from the draft: the absent-minisign test uses a
`BRAID_MINISIGN` env hook instead of PATH exclusion, because on CI's
ubuntu runners apt installs minisign into /usr/bin, inside the
sandbox's SYSTEM_PATH (the hook is no weaker than PATH, which that
attacker already controls).

Extend `crates/braid/tests/installer.rs`:

- [x] Test helpers: locate host minisign (tests need it on the *host*
      PATH even though the sandbox PATH is minimal); generate a
      throwaway keypair per test dir (`minisign -G -W`); sign fixtures
      with trusted comment = archive filename; expose minisign to the
      sandbox by appending its bin dir to the sandbox PATH
- [x] If minisign is missing on the host: tests fail with a clear
      "install minisign" message (not skip — CI installs it; a silent
      skip would unguard the contract). shellcheck's skip-if-missing
      precedent does NOT apply here.
- [x] `--minisign-pubkey` flag (test/fork override of the pinned key) —
      added to help text (help test enumerates flags)
- [x] signed artifact + correct key → installs; stderr says signature verified
- [x] tampered archive (re-tar after signing, checksum updated to match)
      → fails, nothing installed
- [x] missing `.minisig` sidecar → refusal naming `--insecure-skip-signature`
- [x] `--insecure-skip-signature` + missing sidecar → installs with loud
      UNVERIFIED warning (checksum still enforced)
- [x] minisign absent from sandbox PATH → refusal with install guidance
      (and naming the escape hatch)
- [x] trusted-comment mismatch (sign a file under a different name) →
      fails, nothing installed
- [x] wrong public key → fails
- [x] update `help_lists_every_flag_and_exits_zero` for the new flags

## Phase 2 — install.sh

- [x] Pin the release public key in a `MINISIGN_PUBKEY` variable
      (overridable via `--minisign-pubkey`)
- [x] After checksum verification: download `${url}.minisig`, run
      `minisign -Vm <archive> -P <pubkey>`, then compare the verified
      trusted comment against the expected archive filename
- [x] Fail closed on: missing minisign tool (per-platform install
      one-liners), missing sidecar, bad signature, comment mismatch
- [x] `--insecure-skip-signature` escape hatch with loud warning;
      signature steps skipped, checksum path unchanged
- [x] Update usage text; keep shellcheck clean

## Phase 3 — CI: signing + test deps

- [x] release.yml: install minisign per runner (apt on ubuntu/musl
      jobs' distro, brew on macOS); sign each archive
      (`minisign -Sm "$ARCHIVE" -s <tempfile from secret> -t "$ARCHIVE"`),
      temp key file chmod 600 + trap-deleted (beads_rust pattern)
- [x] release.yml: upload `.minisig` next to `.tar.gz`/`.sha256`; release
      job validates all four platforms have all three files; attach to release
- [x] release.yml: update the header contract comment and release-notes
      template (verify instructions + public key)
- [x] ci.yml: install minisign on all three test jobs (ubuntu, macos, musl)

## Phase 4 — docs + wrap-up

- [x] README install section: signature verification, pinned public key,
      "signing key since v0.2.1" note, escape-hatch documentation
- [x] check agents-info.md for installer mentions needing updates
- [ ] `cargo xtask ci` green
- [ ] ask Carlos to push; cut v0.2.1 (braid-release skill) so a signed
      release exists for the strict installer
- [ ] after release: run the ignored `resolves_latest_version_from_github`
      test as a live end-to-end check
- [ ] close br-dgvi0nme with outcome comment

## Notes

- Rotation story (documented in README): generate a new pair, update the
  pinned key on main, future releases sign with it; old releases keep
  old signatures.
- Sandbox PATH in installer.rs is `/usr/bin:/bin:/usr/sbin:/sbin` —
  minisign visibility is therefore *opt-in per test*, which is exactly
  what the absent-minisign test needs.
- minisign's default trusted comment would include a timestamp
  (`timestamp:... file:... hashed`); we override with `-t` for a
  deterministic exact-match check in shell.
