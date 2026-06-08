# Scoop manifest for braid

`braid.json` is the [Scoop](https://scoop.sh) manifest for installing braid
on Windows from the signed GitHub release zip
(`braid-<version>-windows_amd64.zip`).

## Status

**Groundwork, not yet live.** The manifest's `version`/`url`/`hash` are
placeholders (`0.0.0`, empty hash) and stay non-installable until the first
braid release that publishes a Windows artifact (release.yml builds
`windows_amd64` as of this change, so the *next* tagged release will).

The `autoupdate` block is wired against the release URL pattern and the
published `.sha256` file, so the live values can be filled by:

- `scoop update braid.json` locally (Scoop's manifest updater), or
- a release-time workflow that bumps `version` and pins `hash` (beads_rust's
  `update-package-manifests.yml` is the reference for automating this).

## Going live (once a Windows release exists)

1. Fill `version` + the `64bit` `url`/`hash` for that release (or run the
   updater).
2. Host the manifest in a Scoop bucket so users can add it, e.g.
   `scoop bucket add braid https://github.com/cscheid/scoop-braid` then
   `scoop install braid`. (A bucket is just a git repo of manifests.)

Until then, Windows users install via `install.ps1` (see the README) or
`cargo install --git`.
