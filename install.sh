#!/usr/bin/env bash
# omatach installer. Picks the release build for this machine, verifies it
# against its published SHA-256, and installs it under a prefix you own.
#
#   curl -fsSL https://raw.githubusercontent.com/greyhorsesoftware/omatach/main/install.sh | bash
#
# Flags go through the pipe with `bash -s --`:
#
#   curl -fsSL https://raw.githubusercontent.com/greyhorsesoftware/omatach/main/install.sh | bash -s -- --prefix /usr/local
#
# On Arch and Omarchy prefer the package: `yay -S omatach-bin`, or the Omarchy
# menu → Install → Package. This script is for every other Linux.
#
# Every statement lives in a function and `main` runs on the last line, so a
# truncated download cannot execute half an installer.
set -euo pipefail

# Overridable so a fork, a mirror or a pre-release can be installed without editing this.
REPO="${OMATACH_REPO:-greyhorsesoftware/omatach}"
# The release builder runs on ubuntu-22.04, which sets the floor for both arches.
GLIBC_FLOOR="2.35"

PREFIX="${HOME}/.local"
VERSION=""
ACTION="install"
# Set once a download starts; the EXIT trap in main() is the only thing that
# removes it, so it has to outlive do_install's scope.
WORK=""

# ── output ────────────────────────────────────────────────────────────────────
info() { printf '  %s\n' "$*"; }
step() { printf '\n\033[1m%s\033[0m\n' "$*"; }
warn() { printf '\n\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\n\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

usage() {
  cat <<'USAGE'
omatach installer

  install.sh [--prefix DIR] [--version X.Y.Z] [--uninstall] [--help]

  --prefix DIR     install root (default: ~/.local, no sudo needed)
                   the binary lands in DIR/bin, the launcher in
                   DIR/share/applications, the docs in DIR/share/doc/omatach
  --version X.Y.Z  install this release instead of the latest one
  --uninstall      remove the files this script installs, under the same --prefix
  --help           this text

Through a pipe, pass flags after `bash -s --`:

  curl -fsSL <url>/install.sh | bash -s -- --prefix /usr/local

Environment: OMATACH_REPO overrides the GitHub repo releases are fetched from.
USAGE
}

# ── preflight ─────────────────────────────────────────────────────────────────
parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --prefix)  [ $# -ge 2 ] || die "--prefix needs a directory"; PREFIX="${2%/}"; shift 2 ;;
      --prefix=*) PREFIX="${1#*=}"; PREFIX="${PREFIX%/}"; shift ;;
      --version) [ $# -ge 2 ] || die "--version needs a version"; VERSION="${2#v}"; shift 2 ;;
      --version=*) VERSION="${1#*=}"; VERSION="${VERSION#v}"; shift ;;
      --uninstall) ACTION="uninstall"; shift ;;
      -h|--help) usage; exit 0 ;;
      *) usage >&2; die "unknown option: $1" ;;
    esac
  done
  [ -n "$PREFIX" ] || die "--prefix cannot be empty"
  case "$PREFIX" in /*|"$HOME"/*) ;; *) die "--prefix must be an absolute path (got: $PREFIX)" ;; esac
}

need_tools() {
  have tar || die "tar is required"
  if have curl; then FETCH="curl -fsSL -o"; FETCH_OUT="curl -fsSL"
  elif have wget; then FETCH="wget -qO"; FETCH_OUT="wget -qO-"
  else die "need curl or wget to download the release"; fi
  if have sha256sum; then SHA_CHECK="sha256sum -c --status"
  elif have shasum; then SHA_CHECK="shasum -a 256 -c --status"
  else die "need sha256sum or shasum to verify the download"; fi
}

fetch() { # fetch <url> <dest>
  $FETCH "$2" "$1" || die "download failed: $1"
}

fetch_stdout() { # fetch_stdout <url>
  $FETCH_OUT "$1" || die "download failed: $1"
}

cleanup() {
  [ -n "$WORK" ] && rm -rf "$WORK"
  return 0   # never let cleanup decide the script's exit status
}

detect_platform() {
  [ "$(uname -s)" = "Linux" ] || die "omatach releases are Linux-only (this is $(uname -s)).
  Build from source instead: cargo install --git https://github.com/${REPO}"
  case "$(uname -m)" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) die "no release build for $(uname -m) — only x86_64 and aarch64.
  Build from source instead: cargo install --git https://github.com/${REPO}" ;;
  esac
}

# Refuse before writing anything, rather than installing a binary that cannot start.
check_glibc() {
  local out ver
  if ! out="$(ldd --version 2>&1 | head -1)"; then out=""; fi
  case "$out" in
    *musl*) die "this looks like a musl system; the release binaries are built against glibc.
  Build from source instead: cargo install --git https://github.com/${REPO}" ;;
  esac
  ver="$(printf '%s' "$out" | grep -oE '[0-9]+\.[0-9]+' | tail -1)"
  if [ -z "$ver" ]; then
    warn "could not determine the glibc version; continuing (omatach needs ${GLIBC_FLOOR} or newer)"
    return
  fi
  if [ "$(printf '%s\n%s\n' "$GLIBC_FLOOR" "$ver" | sort -V | head -1)" != "$GLIBC_FLOOR" ]; then
    die "glibc ${ver} is older than the ${GLIBC_FLOOR} the release builds need.
  Build from source instead: cargo install --git https://github.com/${REPO}"
  fi
  info "glibc ${ver} (needs ${GLIBC_FLOOR}+)"
}

resolve_version() {
  if [ -n "$VERSION" ]; then info "version ${VERSION} (pinned)"; return; fi
  local api
  api="https://api.github.com/repos/${REPO}/releases/latest"
  VERSION="$(fetch_stdout "$api" | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v?([^"]+)".*/\1/')"
  [ -n "$VERSION" ] || die "could not read the latest version from ${api} — pass --version X.Y.Z"
  info "version ${VERSION} (latest)"
}

# ── install / uninstall ───────────────────────────────────────────────────────
installed_paths() {
  printf '%s\n' \
    "${PREFIX}/bin/omatach" \
    "${PREFIX}/share/applications/omatach.desktop" \
    "${PREFIX}/share/doc/omatach/README.md" \
    "${PREFIX}/share/doc/omatach/LICENSE-MIT" \
    "${PREFIX}/share/doc/omatach/LICENSE-APACHE"
}

check_writable() {
  local d="$1"
  while [ ! -e "$d" ]; do d="$(dirname "$d")"; done
  [ -w "$d" ] || die "${PREFIX} is not writable by $(id -un).
  Either pick a prefix you own (--prefix \"\$HOME/.local\") or re-run this with sudo:
    curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash -s -- --prefix ${PREFIX}"
}

do_install() {
  detect_platform
  check_glibc
  check_writable "$PREFIX"
  resolve_version

  local name tarball sha
  name="omatach-${VERSION}-${ARCH}-linux"
  tarball="${name}.tar.gz"
  sha="${tarball}.sha256"
  WORK="$(mktemp -d)"

  step "Downloading ${tarball}"
  local base="https://github.com/${REPO}/releases/download/v${VERSION}"
  fetch "${base}/${tarball}" "${WORK}/${tarball}"
  fetch "${base}/${sha}"     "${WORK}/${sha}"

  step "Verifying SHA-256"
  # The .sha256 names the tarball with no path, so check from inside the work dir.
  ( cd "$WORK" && $SHA_CHECK "$sha" ) || die "checksum mismatch on ${tarball} — refusing to install.
  Delete nothing and report this: https://github.com/${REPO}/issues"
  info "ok — $(cut -c1-16 <"${WORK}/${sha}")…"

  tar -C "$WORK" -xzf "${WORK}/${tarball}"
  [ -x "${WORK}/${name}/omatach" ] || die "the tarball did not contain an omatach binary"

  step "Installing to ${PREFIX}"
  install -Dm755 "${WORK}/${name}/omatach"         "${PREFIX}/bin/omatach"
  install -Dm644 "${WORK}/${name}/omatach.desktop" "${PREFIX}/share/applications/omatach.desktop"
  install -Dm644 "${WORK}/${name}/README.md"       "${PREFIX}/share/doc/omatach/README.md"
  install -Dm644 "${WORK}/${name}/LICENSE-MIT"     "${PREFIX}/share/doc/omatach/LICENSE-MIT"
  install -Dm644 "${WORK}/${name}/LICENSE-APACHE"  "${PREFIX}/share/doc/omatach/LICENSE-APACHE"
  installed_paths | sed 's/^/  /'
  have update-desktop-database && update-desktop-database "${PREFIX}/share/applications" 2>/dev/null || true

  step "omatach ${VERSION} installed"
  case ":${PATH}:" in
    *":${PREFIX}/bin:"*) info "run: omatach" ;;
    *) warn "${PREFIX}/bin is not on your PATH. Add it:

    echo 'export PATH=\"${PREFIX}/bin:\$PATH\"' >> ~/.bashrc && exec bash

  or run it by its full path: ${PREFIX}/bin/omatach" ;;
  esac
  info "no adapter to hand? pick the Demo adapter in the picker to try every screen"
  info "Bluetooth adapters need bluez running (USB and WiFi do not)"
}

do_uninstall() {
  step "Removing omatach from ${PREFIX}"
  local found=0 p
  while IFS= read -r p; do
    if [ -e "$p" ]; then rm -f "$p" && info "removed ${p}" && found=1; fi
  done < <(installed_paths)
  rmdir "${PREFIX}/share/doc/omatach" 2>/dev/null || true
  # Rebuild the desktop cache so it stops advertising the launcher we just removed.
  # The cache belongs to the whole directory, so only clear it if nothing else is left.
  local apps="${PREFIX}/share/applications"
  if [ -d "$apps" ]; then
    have update-desktop-database && update-desktop-database "$apps" 2>/dev/null || true
    if [ -z "$(find "$apps" -maxdepth 1 -name '*.desktop' -print -quit 2>/dev/null)" ]; then
      rm -f "${apps}/mimeinfo.cache"
      rmdir "$apps" 2>/dev/null || true
    fi
  fi
  if [ "$found" -eq 0 ]; then
    info "nothing installed under ${PREFIX}"
  else
    info "done — config and recordings under ~/.config/omatach and ~/.local/share/omatach were left alone"
  fi
}

main() {
  trap cleanup EXIT
  parse_args "$@"
  need_tools
  case "$ACTION" in
    install)   do_install ;;
    uninstall) do_uninstall ;;
  esac
}

main "$@"
