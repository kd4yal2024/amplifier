#!/usr/bin/env bash
set -euo pipefail

# Installs the tci-client as /usr/local/bin/amplifier-tci-follow
# and creates a systemd service + config file:
#   /etc/amplifier/tci-follow.env  (contains TCI_URL=ws://IP:PORT)
#
# Mutual exclusion: service Conflicts with rigctld (either your custom unit or generic)
#
# Usage:
#   ./install-tci-follow-service.sh
#   ./install-tci-follow-service.sh --url ws://192.168.0.108:50001 --enable --start
#
# Disable:
#   sudo systemctl stop amplifier-tci-follow
#   sudo systemctl disable amplifier-tci-follow

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TCI_DIR="${REPO_ROOT}/tci-client"

BIN_NAME="amplifier-tci-follow"
BIN_DST="/usr/local/bin/${BIN_NAME}"

ENV_DIR="/etc/amplifier"
ENV_FILE="${ENV_DIR}/tci-follow.env"

VALIDATE_DST="/usr/local/bin/${BIN_NAME}-validate"
UNIT_FILE="/etc/systemd/system/${BIN_NAME}.service"
INSTALL_USER="${INSTALL_USER:-${SUDO_USER:-pi}}"

URL=""
DO_ENABLE=0
DO_START=0

die() { echo "ERROR: $*" >&2; exit 1; }
log() { echo "[*] $*"; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --url)
      [[ $# -ge 2 ]] || die "--url requires a value"
      URL="${2:-}"
      shift 2
      ;;
    --enable) DO_ENABLE=1; shift ;;
    --start) DO_START=1; shift ;;
    -h|--help)
      cat <<EOF
Usage:
  $0 [--url ws://IP:PORT] [--enable] [--start]

Examples:
  $0
  $0 --url ws://192.168.0.108:50001 --enable --start
EOF
      exit 0
      ;;
    *) die "Unknown arg: $1" ;;
  esac
done

command -v sudo >/dev/null 2>&1 || die "sudo not found."
command -v cargo >/dev/null 2>&1 || die "cargo not found. Run scripts/install-rust.sh first."
command -v systemctl >/dev/null 2>&1 || die "systemctl not found."
getent passwd "${INSTALL_USER}" >/dev/null 2>&1 || die "Install user not found: ${INSTALL_USER}"

[[ -d "$TCI_DIR" ]] || die "TCI project not found at: $TCI_DIR (did you run cargo new tci-client?)"

log "Building TCI client (release)..."
pushd "$TCI_DIR" >/dev/null
cargo build --release
[[ -f "target/release/tci-client" ]] || die "Expected build output missing: target/release/tci-client"
popd >/dev/null

log "Installing binary to ${BIN_DST}"
sudo install -m 0755 "${TCI_DIR}/target/release/tci-client" "${BIN_DST}"

log "Creating config dir ${ENV_DIR}"
sudo mkdir -p "${ENV_DIR}"
sudo chmod 0755 "${ENV_DIR}"

if [[ ! -f "${ENV_FILE}" ]]; then
  log "Creating ${ENV_FILE} (empty template)"
  sudo tee "${ENV_FILE}" >/dev/null <<'EOF'
# Amplifier TCI Follow-Me config
# Set to something like:
#   TCI_URL=ws://192.168.0.108:50001
TCI_URL=
EOF
  sudo chmod 0644 "${ENV_FILE}"
fi

if [[ -n "${URL}" ]]; then
  log "Writing TCI_URL into ${ENV_FILE}"
  if sudo grep -q '^TCI_URL=' "${ENV_FILE}"; then
    sudo sed -i -E "s|^TCI_URL=.*$|TCI_URL=${URL}|" "${ENV_FILE}"
  else
    printf 'TCI_URL=%s\n' "${URL}" | sudo tee -a "${ENV_FILE}" >/dev/null
  fi
fi

log "Installing validator ${VALIDATE_DST}"
sudo tee "${VALIDATE_DST}" >/dev/null <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

ENV_FILE="/etc/amplifier/tci-follow.env"
[[ -f "$ENV_FILE" ]] || { echo "Missing $ENV_FILE"; exit 2; }

TCI_URL="$(grep -E '^TCI_URL=' "$ENV_FILE" | tail -n1 | cut -d= -f2- | tr -d '\r')"

if [[ -z "${TCI_URL:-}" ]]; then
  echo "TCI_URL is empty. Set TCI_URL=ws://IP:PORT in $ENV_FILE"
  exit 3
fi

if [[ ! "$TCI_URL" =~ ^wss?://([^/:]+):([0-9]{1,5})$ ]]; then
  echo "TCI_URL must look like ws://IP:PORT (got: $TCI_URL)"
  exit 4
fi

host="${BASH_REMATCH[1]}"
port="${BASH_REMATCH[2]}"

# numeric range check
if (( port < 1 || port > 65535 )); then
  echo "Port out of range (1-65535): $port"
  exit 5
fi

# Optional connectivity check (fast fail if the server is wrong/down)
if command -v timeout >/dev/null 2>&1; then
  if ! timeout 2 bash -lc "exec 3<>/dev/tcp/${host}/${port}" 2>/dev/null; then
    echo "Cannot connect to ${host}:${port} (TCI server unreachable?)"
    exit 6
  fi
fi

exit 0
EOF
sudo chmod 0755 "${VALIDATE_DST}"

log "Writing systemd unit ${UNIT_FILE}"
sudo tee "${UNIT_FILE}" >/dev/null <<EOF
[Unit]
Description=Amplifier TCI Follow-Me client
After=network-online.target
Wants=network-online.target

# Don't run alongside Hamlib rig control
Conflicts=amplifier-rigctld.service rigctld.service

[Service]
Type=simple
User=${INSTALL_USER}
EnvironmentFile=${ENV_FILE}

# Refuse to start if URL missing/invalid/unreachable
ExecStartPre=${VALIDATE_DST}

# Run the TCI client; it reads ws://IP:PORT from env
ExecStart=${BIN_DST} \${TCI_URL}

Restart=always
RestartSec=2
RestartPreventExitStatus=2 3 4 5 6
StandardOutput=journal
StandardError=journal
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=full
ProtectHome=true

[Install]
WantedBy=multi-user.target
EOF

log "Reloading systemd"
sudo systemctl daemon-reload

if (( DO_ENABLE )); then
  log "Enabling ${BIN_NAME}.service"
  sudo systemctl enable "${BIN_NAME}.service"
fi

if (( DO_START )); then
  log "Stopping rigctld if running (best-effort), then starting ${BIN_NAME}"
  sudo systemctl stop amplifier-rigctld.service rigctld.service >/dev/null 2>&1 || true
  sudo systemctl start "${BIN_NAME}.service"
  log "Status:"
  sudo systemctl --no-pager --full status "${BIN_NAME}.service" || true
fi

log "Done."
log "Edit ${ENV_FILE} to set TCI_URL, then: sudo systemctl enable --now ${BIN_NAME}.service"
