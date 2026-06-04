#!/usr/bin/env bash
#
# braid installer — downloads a release binary, verifies its checksum,
# and installs it atomically.
#
# One-liner:
#   curl -fsSL https://raw.githubusercontent.com/cscheid/braid/main/install.sh | bash
#
# Pass options after `bash -s --`:
#   curl -fsSL .../install.sh | bash -s -- --dest ~/bin
#
# Design notes (claude-notes/plans/2026/06/04/installer.md):
#   - Non-interactive by construction: the script never reads stdin, so
#     it is safe under `curl | bash` and in CI, and needs none of the
#     piped-stdin re-exec machinery larger installers carry.
#   - Checksum verification is mandatory. Installing an unverified
#     binary requires the explicit --insecure-skip-checksum flag.
#   - Linux binaries are statically linked against musl, so one artifact
#     per architecture covers every distro, Alpine included.
#   - Tested by crates/braid/tests/installer.rs, offline, through
#     --artifact-url file:// + --checksum.

set -euo pipefail
umask 022

# ============================================================================
# Configuration
# ============================================================================
OWNER="${BRAID_REPO_OWNER:-cscheid}"
REPO="${BRAID_REPO_NAME:-braid}"
BINARY_NAME="braid"

VERSION=""
DEST=""
ARTIFACT_URL=""
CHECKSUM=""
INSECURE_SKIP_CHECKSUM=0
FROM_SOURCE=0
UNINSTALL=0
PRINT_PLATFORM=0
QUIET=0

MAX_RETRIES=3
DOWNLOAD_TIMEOUT=120

# ============================================================================
# Output: progress to stderr (stdout stays clean for scripting); colors
# only when stderr is a terminal. --quiet silences progress, never
# warnings or errors.
# ============================================================================
if [ -t 2 ]; then
    RED=$'\033[0;31m' GREEN=$'\033[0;32m' YELLOW=$'\033[1;33m' BLUE=$'\033[0;34m' NC=$'\033[0m'
else
    RED="" GREEN="" YELLOW="" BLUE="" NC=""
fi

log_step()    { [ "$QUIET" -eq 1 ] || printf '%s\n' "${BLUE}→${NC} $*" >&2; }
log_success() { [ "$QUIET" -eq 1 ] || printf '%s\n' "${GREEN}✓${NC} $*" >&2; }
log_warn()    { printf '%s\n' "${YELLOW}braid installer:${NC} $*" >&2; }
log_error()   { printf '%s\n' "${RED}braid installer:${NC} $*" >&2; }
die()         { log_error "$@"; exit 1; }

usage() {
    cat <<EOF
braid installer — install the braid binary from GitHub releases

Usage:
  curl -fsSL https://raw.githubusercontent.com/${OWNER}/${REPO}/main/install.sh | bash
  curl -fsSL .../install.sh | bash -s -- [OPTIONS]

Options:
  --version vX.Y.Z          Install a specific version (default: latest release)
  --dest DIR                Install directory (default: ~/.local/bin)
  --artifact-url URL        Install from a specific artifact URL (file:// works)
  --checksum SHA256         Expected SHA-256 of the artifact
  --insecure-skip-checksum  Allow installation when no checksum is available
  --from-source             Build with cargo from a fresh clone instead
  --uninstall               Remove the installed binary
  --print-platform          Print the detected platform string and exit
  --quiet                   Suppress progress output (warnings/errors still print)
  --help                    Show this help

Environment:
  BRAID_INSTALL_DIR         Override the default install directory
                            (the --dest flag wins over it)

Supported platforms: linux_amd64, linux_arm64 (static musl), darwin_amd64,
darwin_arm64. On anything else, use: cargo install --git https://github.com/${OWNER}/${REPO}
EOF
}

# ============================================================================
# Argument parsing. Unknown flags are errors: a typo silently ignored is
# how a --checksun install ends up unverified.
# ============================================================================
need_value() { [ "$2" -ge 2 ] || die "$1 needs a value (see --help)"; }

while [ $# -gt 0 ]; do
    case "$1" in
        --version)   need_value "$1" $#; VERSION="$2"; shift 2 ;;
        --version=*) VERSION="${1#*=}"; shift ;;
        --dest)      need_value "$1" $#; DEST="$2"; shift 2 ;;
        --dest=*)    DEST="${1#*=}"; shift ;;
        --artifact-url)   need_value "$1" $#; ARTIFACT_URL="$2"; shift 2 ;;
        --artifact-url=*) ARTIFACT_URL="${1#*=}"; shift ;;
        --checksum)   need_value "$1" $#; CHECKSUM="$2"; shift 2 ;;
        --checksum=*) CHECKSUM="${1#*=}"; shift ;;
        --insecure-skip-checksum) INSECURE_SKIP_CHECKSUM=1; shift ;;
        --from-source)    FROM_SOURCE=1; shift ;;
        --uninstall)      UNINSTALL=1; shift ;;
        --print-platform) PRINT_PLATFORM=1; shift ;;
        --quiet|-q)       QUIET=1; shift ;;
        -h|--help)        usage; exit 0 ;;
        *) die "unknown option: $1 (see --help)" ;;
    esac
done

# Dest precedence: --dest flag > BRAID_INSTALL_DIR > ~/.local/bin.
if [ -z "$DEST" ]; then
    if [ -n "${BRAID_INSTALL_DIR:-}" ]; then
        DEST="$BRAID_INSTALL_DIR"
    elif [ -n "${HOME:-}" ]; then
        DEST="$HOME/.local/bin"
    else
        die "HOME is not set; pass --dest DIR"
    fi
fi

# ============================================================================
# Platform detection. Linux artifacts are static musl builds, so libc
# detection is unnecessary: os × arch is the whole story.
# ============================================================================
detect_platform() {
    local os arch
    case "$(uname -s)" in
        Linux*)  os="linux" ;;
        Darwin*) os="darwin" ;;
        *) die "unsupported OS: $(uname -s) — try: cargo install --git https://github.com/${OWNER}/${REPO}" ;;
    esac
    case "$(uname -m)" in
        x86_64|amd64)  arch="amd64" ;;
        aarch64|arm64) arch="arm64" ;;
        *) die "unsupported architecture: $(uname -m) — try: cargo install --git https://github.com/${OWNER}/${REPO}" ;;
    esac
    printf '%s_%s\n' "$os" "$arch"
}

# ============================================================================
# Version resolution: GitHub API first, releases/latest redirect as the
# fallback. Failure is an error (no implicit source build: surprising a
# user with a multi-minute compile is worse than asking them to re-run
# with --from-source).
# ============================================================================
resolve_version() {
    [ -n "$VERSION" ] && return 0

    log_step "resolving latest release..."
    local tag=""
    tag=$(curl -fsSL --connect-timeout 10 --max-time 30 \
        -H "Accept: application/vnd.github+json" \
        "https://api.github.com/repos/${OWNER}/${REPO}/releases/latest" 2>/dev/null \
        | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1) || true

    if [ -z "$tag" ]; then
        tag=$(curl -fsSL -o /dev/null -w '%{url_effective}' \
            "https://github.com/${OWNER}/${REPO}/releases/latest" 2>/dev/null \
            | sed 's|.*/tag/||') || true
    fi

    case "$tag" in
        v[0-9]*) VERSION="$tag"; log_step "latest release: $VERSION" ;;
        *) die "could not determine the latest release; pass --version vX.Y.Z or --from-source" ;;
    esac
}

# ============================================================================
# Download: curl only (preinstalled on macOS, universal on Linux dev
# boxes; a wget fallback would double every download path). Partial
# downloads land in a .part file and are moved into place only on
# success. curl natively honors HTTPS_PROXY et al.
# ============================================================================
download_file() {
    local url="$1" dest="$2" attempt=1
    command -v curl >/dev/null 2>&1 || die "curl is required (https://curl.se)"

    while :; do
        if curl -fL --silent --show-error --retry 2 \
            --connect-timeout 30 --max-time "$DOWNLOAD_TIMEOUT" \
            -o "${dest}.part" "$url" 2>/dev/null; then
            mv -f "${dest}.part" "$dest"
            return 0
        fi
        rm -f "${dest}.part"
        [ "$attempt" -ge "$MAX_RETRIES" ] && return 1
        attempt=$((attempt + 1))
        log_step "download failed; retrying ($attempt/$MAX_RETRIES)..."
        sleep 2
    done
}

# ============================================================================
# Checksum verification: fail closed. A missing checksum aborts the
# install unless --insecure-skip-checksum says otherwise, explicitly.
# ============================================================================
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        return 1
    fi
}

verify_checksum() {
    local file="$1" expected="$2" name="$3"

    if [ -z "$expected" ]; then
        if [ "$INSECURE_SKIP_CHECKSUM" -eq 1 ]; then
            log_warn "no checksum available for $name; installing UNVERIFIED (--insecure-skip-checksum)"
            return 0
        fi
        die "no checksum available for $name; refusing to install an unverified binary.
  Pass --checksum SHA256, provide ${name}.sha256 next to the artifact,
  or (not recommended) re-run with --insecure-skip-checksum."
    fi

    case "$expected" in
        *[!0-9a-fA-F]*) die "invalid SHA-256 checksum: $expected" ;;
    esac
    [ "${#expected}" -eq 64 ] || die "invalid SHA-256 checksum (need 64 hex digits): $expected"

    local actual
    if ! actual=$(sha256_of "$file"); then
        if [ "$INSECURE_SKIP_CHECKSUM" -eq 1 ]; then
            log_warn "no SHA-256 tool found; installing UNVERIFIED (--insecure-skip-checksum)"
            return 0
        fi
        die "no SHA-256 tool found (need sha256sum or shasum)"
    fi

    if [ "$expected" != "$actual" ]; then
        die "checksum mismatch for $name:
  expected: $expected
  got:      $actual"
    fi
    log_success "checksum verified"
}

# ============================================================================
# Atomic install: write next to the destination, then rename. A crash
# mid-install can never leave a truncated binary on PATH.
# ============================================================================
install_binary() {
    local src="$1"
    mkdir -p "$DEST"
    local tmp_dest="$DEST/$BINARY_NAME.tmp.$$"
    if ! install -m 0755 "$src" "$tmp_dest"; then
        rm -f "$tmp_dest"
        die "failed to write to $DEST (permissions?)"
    fi
    mv -f "$tmp_dest" "$DEST/$BINARY_NAME"
}

# ============================================================================
# Release install: download, verify, extract, install.
# ============================================================================
install_from_artifact() {
    local platform="$1" url archive_name

    if [ -n "$ARTIFACT_URL" ]; then
        url="$ARTIFACT_URL"
        archive_name="$(basename "$ARTIFACT_URL")"
    else
        resolve_version
        local tag="v${VERSION#v}" ver="${VERSION#v}"
        archive_name="${BINARY_NAME}-${ver}-${platform}.tar.gz"
        url="https://github.com/${OWNER}/${REPO}/releases/download/${tag}/${archive_name}"
    fi

    log_step "downloading $archive_name..."
    download_file "$url" "$TMP/$archive_name" || die "download failed: $url"

    local expected=""
    if [ -n "$CHECKSUM" ]; then
        expected="${CHECKSUM%% *}"
    elif download_file "${url}.sha256" "$TMP/expected.sha256"; then
        expected="$(awk '{print $1; exit}' "$TMP/expected.sha256")"
    fi
    verify_checksum "$TMP/$archive_name" "$expected" "$archive_name"

    log_step "extracting..."
    mkdir -p "$TMP/extract"
    tar -xzf "$TMP/$archive_name" -C "$TMP/extract" \
        || die "could not extract $archive_name"

    [ -f "$TMP/extract/$BINARY_NAME" ] \
        || die "archive does not contain a '$BINARY_NAME' binary"

    install_binary "$TMP/extract/$BINARY_NAME"
    log_success "installed $DEST/$BINARY_NAME"
}

# ============================================================================
# Source build: explicit opt-in only. Requires an existing Rust
# toolchain; this script does not install one behind your back.
# ============================================================================
build_from_source() {
    command -v git >/dev/null 2>&1 || die "git is required for --from-source"
    command -v cargo >/dev/null 2>&1 \
        || die "cargo is required for --from-source; install Rust via https://rustup.rs and re-run"

    local clone_args=(--quiet --depth 1)
    [ -n "$VERSION" ] && clone_args+=(--branch "v${VERSION#v}")

    log_step "cloning ${OWNER}/${REPO}..."
    git clone "${clone_args[@]}" "https://github.com/${OWNER}/${REPO}.git" "$TMP/src" \
        || die "clone failed"

    log_step "building with cargo (this may take a few minutes)..."
    (cd "$TMP/src" && CARGO_TARGET_DIR="$TMP/target" cargo build --release --quiet -p braid) \
        || die "build failed"

    [ -f "$TMP/target/release/$BINARY_NAME" ] || die "binary not found after build"
    install_binary "$TMP/target/release/$BINARY_NAME"
    log_success "installed $DEST/$BINARY_NAME (source build)"
}

# ============================================================================
# PATH advice. Printed, never applied: this script does not edit shell
# rc files.
# ============================================================================
warn_path() {
    case ":$PATH:" in
        *:"$DEST":*) ;;
        *) log_warn "$DEST is not on your PATH; add it with:
  export PATH=\"$DEST:\$PATH\"" ;;
    esac
}

do_uninstall() {
    if [ -f "$DEST/$BINARY_NAME" ]; then
        rm -f "$DEST/$BINARY_NAME"
        log_success "removed $DEST/$BINARY_NAME"
    else
        log_warn "nothing to remove at $DEST/$BINARY_NAME"
    fi
}

# ============================================================================
# Main
# ============================================================================
TMP=""
cleanup() { [ -n "$TMP" ] && rm -rf "$TMP"; return 0; }
trap cleanup EXIT

main() {
    if [ "$PRINT_PLATFORM" -eq 1 ]; then
        detect_platform
        exit 0
    fi
    if [ "$UNINSTALL" -eq 1 ]; then
        do_uninstall
        exit 0
    fi

    TMP=$(mktemp -d)

    if [ "$FROM_SOURCE" -eq 1 ]; then
        log_step "install directory: $DEST"
        build_from_source
    else
        local platform
        platform=$(detect_platform)
        log_step "platform: $platform"
        log_step "install directory: $DEST"
        install_from_artifact "$platform"
    fi

    warn_path

    local installed_version
    installed_version=$("$DEST/$BINARY_NAME" --version 2>/dev/null || echo "unknown")
    log_success "done: $installed_version"
}

# The braces make bash parse this whole block before executing it, so a
# `curl | bash` download truncated mid-script can never run half of main.
{ main "$@"; }
