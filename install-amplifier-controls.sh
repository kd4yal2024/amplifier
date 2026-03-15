#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MAIN_BIN_NAME="amplifier"
MAIN_BIN_DST="/usr/local/bin/${MAIN_BIN_NAME}"
SERVICE_NAME="amplifier.service"
DEFAULT_BIND_ADDR="0.0.0.0:3000"
HTTP_PORT="${DEFAULT_BIND_ADDR##*:}"
FORCE=0
INSTALL_USER="${INSTALL_USER:-${SUDO_USER:-pi}}"
INSTALL_HOME=""

log() { printf "\n[%s] %s\n" "$(date +'%F %T')" "$*"; }
die() { printf "\nERROR: %s\n" "$*" >&2; exit 1; }

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

resolve_install_user() {
  local passwd_entry
  passwd_entry="$(getent passwd "${INSTALL_USER}" || true)"
  [[ -n "${passwd_entry}" ]] || die "Install user not found: ${INSTALL_USER}"
  INSTALL_HOME="$(printf '%s\n' "${passwd_entry}" | cut -d: -f6)"
  [[ -n "${INSTALL_HOME}" ]] || die "Could not resolve home directory for ${INSTALL_USER}"
}

ensure_labwc_touch_entry() {
  local file="$1"
  local entry="$2"
  local tmp

  grep -Fqx "$entry" "$file" && return 0

  tmp="$(mktemp)"
  awk -v entry="$entry" '
    /<\/openbox_config>/ && !inserted {
      print entry
      inserted=1
    }
    { print }
    END {
      if (!inserted) {
        print entry
      }
    }
  ' "$file" >"$tmp"
  mv "$tmp" "$file"
}

install_labwc_touch_mappings() {
  local user_rc="${INSTALL_HOME}/.config/labwc/rc.xml"
  local system_rc="/etc/xdg/labwc/rc.xml"
  local entries=(
    '  <touch deviceName="10-0014 Goodix Capacitive TouchScreen" mapToOutput="DSI-1" mouseEmulation="yes" />'
    '  <touch deviceName="11-0014 Goodix Capacitive TouchScreen" mapToOutput="DSI-2" mouseEmulation="yes" />'
    '  <touch deviceName="6-0014 Goodix Capacitive TouchScreen" mapToOutput="DSI-1" mouseEmulation="yes" />'
    '  <touch deviceName="4-0014 Goodix Capacitive TouchScreen" mapToOutput="DSI-2" mouseEmulation="yes" />'
  )

  mkdir -p "$(dirname "${user_rc}")"
  if [[ ! -f "${user_rc}" ]]; then
    if [[ -f "${system_rc}" ]]; then
      cp "${system_rc}" "${user_rc}"
    else
      cat >"${user_rc}" <<'EOF'
<openbox_config>
</openbox_config>
EOF
    fi
  fi

  for entry in "${entries[@]}"; do
    ensure_labwc_touch_entry "${user_rc}" "${entry}"
  done

  if command -v labwc >/dev/null 2>&1; then
    local labwc_pid
    labwc_pid="$(pgrep -u "${INSTALL_USER}" -x labwc | head -n1 || true)"
    if [[ -n "${labwc_pid}" ]]; then
      sudo -u "${INSTALL_USER}" env LABWC_PID="${labwc_pid}" labwc --reconfigure || true
    fi
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force) FORCE=1; shift ;;
    -h|--help)
      cat <<EOF
Usage:
  $0 [--force]

Run from the repository root checkout you want to install.

Options:
  --force    Continue even if the repo checkout has uncommitted changes.
EOF
      exit 0
      ;;
    *) die "Unknown arg: $1" ;;
  esac
done

require_cmd git
require_cmd cargo
require_cmd sudo
require_cmd systemctl
require_cmd ss
require_cmd getent

[[ -d "${REPO_ROOT}/.git" ]] || die "Run this script from the repository root checkout."
resolve_install_user

if [[ "${FORCE}" -eq 0 ]] && [[ -n "$(git -C "${REPO_ROOT}" status --porcelain)" ]]; then
  die "Repo has uncommitted changes. Commit/stash them or rerun with --force."
fi

cd "${REPO_ROOT}"

log "Ensuring helper scripts are executable..."
chmod +x scripts/*.sh

log "Installing labwc touchscreen mappings for the Goodix DSI panel..."
install_labwc_touch_mappings
log "labwc touch mappings OK: ${INSTALL_HOME}/.config/labwc/rc.xml"

log "Installing Hamlib..."
"${REPO_ROOT}/scripts/install-hamlib.sh"
if ! rigctl -V >/dev/null 2>&1; then
  die "Hamlib validation failed: rigctl not available."
fi
log "Hamlib OK: $(rigctl -V | head -n1)"

log "Installing Rust toolchain..."
"${REPO_ROOT}/scripts/install-rust.sh"
source "${HOME}/.cargo/env"
if ! cargo --version >/dev/null 2>&1; then
  die "Rust validation failed: cargo not available."
fi
log "Rust OK: $(cargo --version)"

log "Installing TCI follow service..."
"${REPO_ROOT}/scripts/install-tci-follow-service.sh"
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
User=${INSTALL_USER}
WorkingDirectory=${REPO_ROOT}
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
log "Repo: ${REPO_ROOT}"
log "Binary: ${MAIN_BIN_DST}"
log "Service: ${SERVICE_NAME}"
log "HTTP port: ${HTTP_PORT}"
