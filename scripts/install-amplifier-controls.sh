#!/usr/bin/env bash
set -euo pipefail

REPO_URL="https://github.com/kd4yal2024/amplifier"
BRANCH="follow-me"
REPO_NAME="amplifier"
MAIN_BIN_NAME="amplifier"
MAIN_BIN_DST="/usr/local/bin/${MAIN_BIN_NAME}"
SERVICE_NAME="amplifier.service"
DEFAULT_BIND_ADDR="0.0.0.0:3000"
HTTP_PORT="${DEFAULT_BIND_ADDR##*:}"
FORCE=0

log() { printf "\n[%s] %s\n" "$(date +'%F %T')" "$*"; }
die() { printf "\nERROR: %s\n" "$*" >&2; exit 1; }

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force) FORCE=1; shift ;;
    -h|--help)
      cat <<EOF
Usage:
  $0 [--force]

Options:
  --force    Continue even if the target repo checkout has uncommitted changes.
EOF
      exit 0
      ;;
    *) die "Unknown arg: $1" ;;
  esac
done

read -r -p "Install directory (repo will be placed under this directory): " INSTALL_ROOT
[[ -n "${INSTALL_ROOT}" ]] || die "No install directory provided."

INSTALL_ROOT="$(realpath -m "${INSTALL_ROOT}")"
TARGET_DIR="${INSTALL_ROOT}/${REPO_NAME}"

log "Preparing install directory: ${INSTALL_ROOT}"
mkdir -p "${INSTALL_ROOT}"

require_cmd git
require_cmd cargo
require_cmd sudo
require_cmd systemctl
require_cmd ss

if [[ -d "${TARGET_DIR}/.git" ]]; then
  log "Repo exists. Updating ${TARGET_DIR}..."
  if [[ "${FORCE}" -eq 0 ]] && [[ -n "$(git -C "${TARGET_DIR}" status --porcelain)" ]]; then
    die "Target repo has uncommitted changes. Commit/stash them or rerun with --force."
  fi
  CURRENT_BRANCH="$(git -C "${TARGET_DIR}" branch --show-current)"
  git -C "${TARGET_DIR}" fetch origin
  if [[ "${CURRENT_BRANCH}" == "${BRANCH}" ]]; then
    git -C "${TARGET_DIR}" pull --ff-only origin "${BRANCH}"
  else
    log "Leaving existing checkout on branch: ${CURRENT_BRANCH:-detached HEAD}"
    log "Skipping branch switch to ${BRANCH}; use a fresh install directory to clone that branch."
  fi
else
  log "Cloning ${REPO_URL} (${BRANCH}) into ${TARGET_DIR}..."
  git clone -b "${BRANCH}" "${REPO_URL}" "${TARGET_DIR}"
fi

cd "${TARGET_DIR}"

log "Ensuring scripts are executable..."
chmod +x scripts/*.sh

log "Installing Hamlib..."
scripts/install-hamlib.sh
if ! rigctl -V >/dev/null 2>&1; then
  die "Hamlib validation failed: rigctl not available."
fi
log "Hamlib OK: $(rigctl -V | head -n1)"

log "Installing Rust toolchain..."
scripts/install-rust.sh
source "${HOME}/.cargo/env"
if ! cargo --version >/dev/null 2>&1; then
  die "Rust validation failed: cargo not available."
fi
log "Rust OK: $(cargo --version)"

log "Installing TCI follow service..."
scripts/install-tci-follow-service.sh
if [[ -f /etc/systemd/system/amplifier-tci-follow.service ]]; then
  log "TCI follow unit installed."
else
  die "TCI follow unit not found after install."
fi
if [[ -x /usr/local/bin/amplifier-tci-follow-validate ]]; then
  if ! /usr/local/bin/amplifier-tci-follow-validate; then
    log "TCI validation failed (likely missing TCI_URL). Update /etc/amplifier/tci-follow.env."
  else
    log "TCI validation OK."
  fi
fi

log "Building amplifier (release)..."
cargo build --release
[[ -f "target/release/amplifier" ]] || die "Build failed: target/release/amplifier not found."
log "Build OK: target/release/amplifier"

log "Installing amplifier binary to ${MAIN_BIN_DST}..."
sudo install -m 0755 "target/release/amplifier" "${MAIN_BIN_DST}"
[[ -x "${MAIN_BIN_DST}" ]] || die "Install failed: ${MAIN_BIN_DST} not executable."
log "Binary install OK: ${MAIN_BIN_DST}"

log "Installing amplifier systemd service..."
sudo tee /etc/systemd/system/${SERVICE_NAME} >/dev/null <<EOF
[Unit]
Description=Amplifier Controls
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=${TARGET_DIR}
ExecStart=${MAIN_BIN_DST}
Restart=always
RestartSec=2
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now "${SERVICE_NAME}"
sleep 2
sudo systemctl --quiet is-active "${SERVICE_NAME}" || {
  sudo systemctl --no-pager --full status "${SERVICE_NAME}" || true
  die "${SERVICE_NAME} did not reach active state."
}
if ! sudo ss -ltnp | grep -q ":${HTTP_PORT}\b"; then
  sudo systemctl --no-pager --full status "${SERVICE_NAME}" || true
  die "${SERVICE_NAME} is active but port ${HTTP_PORT} is not listening."
fi
sudo systemctl --no-pager --full status "${SERVICE_NAME}" || true

log "Install complete."
log "Repo: ${TARGET_DIR}"
log "Binary: ${MAIN_BIN_DST}"
log "Service: ${SERVICE_NAME}"
log "HTTP port: ${HTTP_PORT}"
