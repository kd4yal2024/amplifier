#!/usr/bin/env bash
set -euo pipefail

SERVICE_NAME="amplifier.service"
TCI_SERVICE_NAME="amplifier-tci-follow.service"
MAIN_BIN_DST="/usr/local/bin/amplifier"
TCI_BIN_DST="/usr/local/bin/amplifier-tci-follow"
TCI_VALIDATE_DST="/usr/local/bin/amplifier-tci-follow-validate"
TCI_ENV_FILE="/etc/amplifier/tci-follow.env"
REMOVE_TCI_ENV=0

log() { printf "\n[%s] %s\n" "$(date +'%F %T')" "$*"; }
die() { printf "\nERROR: %s\n" "$*" >&2; exit 1; }

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --remove-tci-env) REMOVE_TCI_ENV=1; shift ;;
    -h|--help)
      cat <<EOF
Usage:
  $0 [--remove-tci-env]

Options:
  --remove-tci-env    Also remove ${TCI_ENV_FILE}.
EOF
      exit 0
      ;;
    *) die "Unknown arg: $1" ;;
  esac
done

require_cmd sudo
require_cmd systemctl

log "Stopping services..."
sudo systemctl stop "${SERVICE_NAME}" "${TCI_SERVICE_NAME}" >/dev/null 2>&1 || true

log "Disabling services..."
sudo systemctl disable "${SERVICE_NAME}" "${TCI_SERVICE_NAME}" >/dev/null 2>&1 || true

log "Removing systemd units..."
sudo rm -f "/etc/systemd/system/${SERVICE_NAME}" "/etc/systemd/system/${TCI_SERVICE_NAME}"
sudo systemctl daemon-reload

log "Removing installed binaries..."
sudo rm -f "${MAIN_BIN_DST}" "${TCI_BIN_DST}" "${TCI_VALIDATE_DST}"

if [[ "${REMOVE_TCI_ENV}" -eq 1 ]]; then
  log "Removing TCI environment file..."
  sudo rm -f "${TCI_ENV_FILE}"
fi

log "Uninstall complete."
log "Removed service units and installed binaries."
