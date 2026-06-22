# Installation

```sh
curl -fsSL https://raw.githubusercontent.com/cscheid/braid/main/install.sh | bash
```

Installs the latest release to `~/.local/bin` after verifying its
SHA-256 checksum **and its Ed25519 signature** (release archives are
signed with [minisign](https://jedisct1.github.io/minisign/), the tool
Zig signs its releases with). Signature verification is mandatory, so
the installer needs minisign present — `brew install minisign`,
`apt install minisign`, `dnf install minisign`, or `apk add minisign`
first. Prebuilt binaries cover Linux x86_64/ARM64 (statically linked —
works on any distro, Alpine included), macOS Intel/Apple Silicon, and
Windows x86_64 (see below). The installer never asks questions and never
edits your shell config; if `~/.local/bin` isn't on your PATH it tells you
the line to add.

The release signing key (since v0.2.1; pinned in `install.sh`, which
ships from the `main` branch — not from the release being verified):

```
RWSbWhSzVkkTRO4nFMzL/KyRs9oicbgy/2KPRK+o9hxznRYx9ZkHwwlN
```

To verify a manually downloaded archive:
`minisign -Vm braid-<version>-<platform>.tar.gz -P <key above>` — the
trusted comment should name exactly the file you downloaded. If the
signing key is ever rotated, the new key lands here and in `install.sh`
in the same commit; releases keep the signatures they shipped with.

Useful flags (pass after `bash -s --`):

```sh
# specific version, custom directory
curl -fsSL .../install.sh | bash -s -- --version v0.2.1 --dest ~/bin

# build from source instead (needs a Rust toolchain)
curl -fsSL .../install.sh | bash -s -- --from-source

# install without signature verification (not recommended; note the
# flag goes after `bash -s --`, not on bash itself)
curl -fsSL .../install.sh | bash -s -- --insecure-skip-signature

# remove an installed binary
curl -fsSL .../install.sh | bash -s -- --uninstall
```

## Windows

```powershell
irm https://raw.githubusercontent.com/cscheid/braid/main/install.ps1 | iex
```

Downloads the latest `braid-<version>-windows_amd64.zip`, verifies its
SHA-256, and installs `braid.exe` to `%USERPROFILE%\.local\bin` (override
with `-Dest`). It prints the line to add that directory to your PATH if it
isn't already. To verify a manual download, the same minisign key and
`.minisig` files apply.

## From source

With a Rust toolchain (any platform):

```sh
cargo install --git https://github.com/cscheid/braid braid
```
