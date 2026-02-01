#!/usr/bin/env bash
set -euo pipefail

# install-hamlib.sh
# Installs Hamlib runtime + rigctl/rigctld utilities.
# Strategy:
#   - If apt can provide libhamlib-utils >= desired version, use apt.
#   - Else build Hamlib from the official release tarball.
#
# Usage:
#   ./install-hamlib.sh
#   ./install-hamlib.sh --method apt
#   ./install-hamlib.sh --method source
#   HAMLIB_VERSION=4.6.5 ./install-hamlib.sh
#
# Notes:
#   - Source installs default to /usr/local (so it won't clobber distro packages).
#   - rigctl/rigctld live in libhamlib-utils on Debian-based distros.

HAMLIB_VERSION="${HAMLIB_VERSION:-4.6.5}"
METHOD="auto"           # auto|apt|source
PREFIX="/usr/local"
KEEP_BUILD=0

log() { printf "\n[%s] %s\n" "$(date +'%F %T')" "$*"; }
die() { printf "\nERROR: %s\n" "$*" >&2; exit 1; }

usage() {
  cat <<EOF
Usage: $0 [--method auto|apt|source] [--prefix /usr/local] [--keep-build]

Environment:
  HAMLIB_VERSION=4.6.5   Desired Hamlib version (default: ${HAMLIB_VERSION})

Examples:
  $0
  $0 --method apt
  $0 --method source --prefix /usr/local
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --method) METHOD="${2:-}"; shift 2 ;;
    --prefix) PREFIX="${2:-}"; shift 2 ;;
    --keep-build) KEEP_BUILD=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown arg: $1" ;;
  esac
done

command -v sudo >/dev/null 2>&1 || die "sudo not found (run as root or install sudo)."
command -v apt-get >/dev/null 2>&1 || die "apt-get not found (this script targets Debian/Raspberry Pi OS)."

version_ge() {
  # dpkg compares Debian version strings correctly (handles 4.6.5-5, etc.)
  dpkg --compare-versions "$1" ge "$2"
}

extract_hamlib_ver_from_rigctl() {
  # Example output observed: "rigctl, Hamlib 3.1~git" or "rigctl, Hamlib 4.6.5"
  rigctl -V 2>/dev/null | head -n1 | sed -n 's/.*Hamlib[[:space:]]\([0-9][0-9.]*\).*/\1/p' || true
}

apt_candidate_ver() {
  apt-cache policy libhamlib-utils 2>/dev/null | awk -F': ' '/Candidate:/ {print $2}' | head -n1
}

apt_install() {
  log "Installing via apt…"
  sudo apt-get update -y
  # libhamlib-utils pulls in the right libhamlib runtime package on that distro
  sudo apt-get install -y libhamlib-utils

  # Optional: explicitly install runtime library if present (harmless if already pulled)
  if apt-cache show libhamlib4t64 >/dev/null 2>&1; then
    sudo apt-get install -y libhamlib4t64 || true
  elif apt-cache show libhamlib4 >/dev/null 2>&1; then
    sudo apt-get install -y libhamlib4 || true
  fi

  log "Installed binaries:"
  command -v rigctl || true
  command -v rigctld || true
  rigctl -V || true
  rigctld -V || true
}

source_install() {
  log "Building Hamlib ${HAMLIB_VERSION} from source…"
  sudo apt-get update -y

  # Build deps (kept intentionally minimal but practical)
  sudo apt-get install -y \
    build-essential pkg-config \
    autoconf automake libtool \
    libusb-1.0-0-dev libreadline-dev \
    ca-certificates curl tar

  local build_root
  build_root="$(mktemp -d -t hamlib-build.XXXXXX)"
  if [[ $KEEP_BUILD -eq 0 ]]; then
    trap "rm -rf '$build_root'" EXIT
  else
    log "Keeping build directory: $build_root"
  fi

  local tarball="hamlib-${HAMLIB_VERSION}.tar.gz"
  local url="https://sourceforge.net/projects/hamlib/files/hamlib/${HAMLIB_VERSION}/${tarball}/download"

  log "Downloading: ${tarball}"
  curl -L --fail --retry 3 --retry-delay 2 -o "${build_root}/${tarball}" "${url}"

  log "Extracting…"
  tar -xzf "${build_root}/${tarball}" -C "$build_root"

  local src_dir="${build_root}/hamlib-${HAMLIB_VERSION}"
  [[ -d "$src_dir" ]] || die "Expected source dir not found: $src_dir"

  log "Configuring (prefix=${PREFIX})…"
  pushd "$src_dir" >/dev/null

  ./configure --prefix="$PREFIX"
  log "Compiling…"
  make -j"$(nproc)"

  log "Installing…"
  sudo make install
  sudo ldconfig

  popd >/dev/null

  log "Installed binaries:"
  command -v rigctl || true
  command -v rigctld || true
  rigctl -V || true
  rigctld -V || true

  log "NOTE: Source install typically lands in ${PREFIX}/bin (often /usr/local/bin)."
  log "If you also have distro hamlib installed, /usr/local/bin should take precedence in PATH."
}

main() {
  log "Desired Hamlib version: ${HAMLIB_VERSION}"
  log "Method: ${METHOD}"

  local installed_ver=""
  installed_ver="$(extract_hamlib_ver_from_rigctl || true)"
  if [[ -n "$installed_ver" ]]; then
    log "Detected installed rigctl Hamlib version: ${installed_ver}"
  else
    log "rigctl not detected (yet)."
  fi

  if [[ "$METHOD" == "apt" ]]; then
    apt_install
    exit 0
  fi

  if [[ "$METHOD" == "source" ]]; then
    source_install
    exit 0
  fi

  # auto mode:
  local cand=""
  cand="$(apt_candidate_ver || true)"

  if [[ -n "$cand" && "$cand" != "(none)" ]]; then
    log "apt Candidate for libhamlib-utils: ${cand}"
    if version_ge "$cand" "$HAMLIB_VERSION"; then
      log "apt can satisfy >= ${HAMLIB_VERSION}; using apt."
      apt_install
      exit 0
    fi
  else
    log "apt Candidate for libhamlib-utils not found (or apt-cache not ready)."
  fi

  log "apt cannot satisfy >= ${HAMLIB_VERSION}; building from source."
  source_install
}

main
