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
INSTALL_GROUP=""
REBOOT_REQUIRED=0

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
  INSTALL_GROUP="$(id -gn "${INSTALL_USER}")"
  [[ -n "${INSTALL_HOME}" ]] || die "Could not resolve home directory for ${INSTALL_USER}"
  [[ -n "${INSTALL_GROUP}" ]] || die "Could not resolve primary group for ${INSTALL_USER}"
}

update_text_file() {
  local path="$1"
  local owner="$2"
  local group="$3"
  local mode="$4"
  local tmp existing

  tmp="$(mktemp)"
  existing="$(mktemp)"
  cat >"${tmp}"
  if sudo test -f "${path}"; then
    sudo cat "${path}" >"${existing}"
  else
    : >"${existing}"
  fi
  if ! cmp -s "${tmp}" "${existing}"; then
    sudo install -d -m 0755 -o "${owner}" -g "${group}" "$(dirname "${path}")"
    sudo install -m "${mode}" -o "${owner}" -g "${group}" "${tmp}" "${path}"
  fi
  rm -f "${tmp}" "${existing}"
}

ensure_root_line() {
  local file="$1"
  local line="$2"
  local tmp existing

  tmp="$(mktemp)"
  existing="$(mktemp)"
  sudo cat "${file}" >"${existing}"
  cat "${existing}" >"${tmp}"
  grep -Fqx "${line}" "${existing}" || printf '%s\n' "${line}" >>"${tmp}"
  if ! cmp -s "${tmp}" "${existing}"; then
    sudo install -m 0644 "${tmp}" "${file}"
    REBOOT_REQUIRED=1
  fi
  rm -f "${tmp}" "${existing}"
}

upsert_root_setting() {
  local file="$1"
  local key="$2"
  local value="$3"
  local tmp existing

  tmp="$(mktemp)"
  existing="$(mktemp)"
  sudo cat "${file}" >"${existing}"
  awk -v key="${key}" -v value="${value}" '
    BEGIN { done=0 }
    $0 ~ "^[[:space:]]*#?[[:space:]]*" key "=" {
      if (!done) {
        print key "=" value
        done=1
      }
      next
    }
    { print }
    END {
      if (!done) {
        print key "=" value
      }
    }
  ' "${existing}" >"${tmp}"
  if ! cmp -s "${tmp}" "${existing}"; then
    sudo install -m 0644 "${tmp}" "${file}"
    REBOOT_REQUIRED=1
  fi
  rm -f "${tmp}" "${existing}"
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
    '  <touch deviceName="10-0038 generic ft5x06 (79)" mapToOutput="DSI-1" mouseEmulation="yes" />'
    '  <touch deviceName="11-0038 generic ft5x06 (79)" mapToOutput="DSI-2" mouseEmulation="yes" />'
    '  <touch deviceName="6-0038 generic ft5x06 (79)" mapToOutput="DSI-1" mouseEmulation="yes" />'
    '  <touch deviceName="4-0038 generic ft5x06 (79)" mapToOutput="DSI-2" mouseEmulation="yes" />'
    '  <touch deviceName="10-0038 generic ft5x06 (00)" mapToOutput="DSI-1" mouseEmulation="yes" />'
    '  <touch deviceName="11-0038 generic ft5x06 (00)" mapToOutput="DSI-2" mouseEmulation="yes" />'
    '  <touch deviceName="6-0038 generic ft5x06 (00)" mapToOutput="DSI-1" mouseEmulation="yes" />'
    '  <touch deviceName="4-0038 generic ft5x06 (00)" mapToOutput="DSI-2" mouseEmulation="yes" />'
    '  <touch deviceName="10-005d Goodix Capacitive TouchScreen" mapToOutput="DSI-1" mouseEmulation="yes" />'
    '  <touch deviceName="11-005d Goodix Capacitive TouchScreen" mapToOutput="DSI-2" mouseEmulation="yes" />'
    '  <touch deviceName="6-005d Goodix Capacitive TouchScreen" mapToOutput="DSI-1" mouseEmulation="yes" />'
    '  <touch deviceName="4-005d Goodix Capacitive TouchScreen" mapToOutput="DSI-2" mouseEmulation="yes" />'
    '  <touch deviceName="10-0014 Goodix Capacitive TouchScreen" mapToOutput="DSI-1" mouseEmulation="yes" />'
    '  <touch deviceName="11-0014 Goodix Capacitive TouchScreen" mapToOutput="DSI-2" mouseEmulation="yes" />'
    '  <touch deviceName="6-0014 Goodix Capacitive TouchScreen" mapToOutput="DSI-1" mouseEmulation="yes" />'
    '  <touch deviceName="4-0014 Goodix Capacitive TouchScreen" mapToOutput="DSI-2" mouseEmulation="yes" />'
  )

  sudo install -d -m 0755 -o "${INSTALL_USER}" -g "${INSTALL_GROUP}" "$(dirname "${user_rc}")"
  if [[ ! -f "${user_rc}" ]]; then
    if [[ -f "${system_rc}" ]]; then
      sudo cp "${system_rc}" "${user_rc}"
      sudo chown "${INSTALL_USER}:${INSTALL_GROUP}" "${user_rc}"
    else
      update_text_file "${user_rc}" "${INSTALL_USER}" "${INSTALL_GROUP}" 0644 <<'EOF'
<openbox_config>
</openbox_config>
EOF
    fi
  fi

  for entry in "${entries[@]}"; do
    ensure_labwc_touch_entry "${user_rc}" "${entry}"
  done

  perl -0pi -e 's{<windowRule identifier="chromium" matchOnce="true">.*?</windowRule>\s*}{}sg' "${user_rc}"
  perl -0pi -e 's{<windowRule identifier="chromium-browser" matchOnce="true">.*?</windowRule>\s*}{}sg' "${user_rc}"

  if grep -q '</windowRules>' "${user_rc}"; then
    awk '
      /<\/windowRules>/ && !inserted {
        print "    <windowRule identifier=\"chromium\" matchOnce=\"true\">"
        print "      <action name=\"Maximize\" />"
        print "    </windowRule>"
        print "    <windowRule identifier=\"chromium-browser\" matchOnce=\"true\">"
        print "      <action name=\"Maximize\" />"
        print "    </windowRule>"
        inserted=1
      }
      { print }
    ' "${user_rc}" >"${user_rc}.tmp"
    mv "${user_rc}.tmp" "${user_rc}"
  else
    awk '
      /<\/openbox_config>/ && !inserted {
        print "  <windowRules>"
        print "    <windowRule identifier=\"chromium\" matchOnce=\"true\">"
        print "      <action name=\"Maximize\" />"
        print "    </windowRule>"
        print "    <windowRule identifier=\"chromium-browser\" matchOnce=\"true\">"
        print "      <action name=\"Maximize\" />"
        print "    </windowRule>"
        print "  </windowRules>"
        inserted=1
      }
      { print }
    ' "${user_rc}" >"${user_rc}.tmp"
    mv "${user_rc}.tmp" "${user_rc}"
  fi

  sudo chown "${INSTALL_USER}:${INSTALL_GROUP}" "${user_rc}"

  if command -v labwc >/dev/null 2>&1; then
    local labwc_pid
    labwc_pid="$(pgrep -u "${INSTALL_USER}" -x labwc | head -n1 || true)"
    if [[ -n "${labwc_pid}" ]]; then
      sudo -u "${INSTALL_USER}" env LABWC_PID="${labwc_pid}" labwc --reconfigure || true
    fi
  fi
}

install_labwc_autostart() {
  local autostart_file="${INSTALL_HOME}/.config/labwc/autostart"
  update_text_file "${autostart_file}" "${INSTALL_USER}" "${INSTALL_GROUP}" 0644 <<'EOF'
# Keep the kiosk session awake. The LCD/touch stack does not recover
# reliably after idle display power-off or system sleep.
systemd-inhibit --what=idle:sleep --who=amplifier-kiosk --why='keep LCD touchscreen awake' sleep infinity &
EOF
}

install_kanshi_layout() {
  local kanshi_dir="${INSTALL_HOME}/.config/kanshi"
  local layout_content

  layout_content="$(cat <<'EOF'
profile {
    output DSI-1 enable mode 1280x800@60.026 position 0,0 scale 1.000000 transform normal
    output HDMI-A-1 enable mode 1920x1080@60.000 position 1280,0 scale 1.000000 transform normal
}
profile {
    output DSI-1 enable mode 1280x800@60.026 position 0,0 scale 1.000000 transform normal
}
profile {
    output HDMI-A-1 enable mode 1920x1080@60.000 position 0,0 scale 1.000000 transform normal
}
EOF
)"

  update_text_file "${kanshi_dir}/config" "${INSTALL_USER}" "${INSTALL_GROUP}" 0644 <<<"${layout_content}"
  update_text_file "${kanshi_dir}/config.init" "${INSTALL_USER}" "${INSTALL_GROUP}" 0644 <<<"${layout_content}"
}

install_kiosk_autostart() {
  local kiosk_file="${INSTALL_HOME}/.config/autostart/amplifier-kiosk.desktop"
  update_text_file "${kiosk_file}" "${INSTALL_USER}" "${INSTALL_GROUP}" 0644 <<EOF
[Desktop Entry]
Type=Application
Name=Amplifier Kiosk
Comment=Open the amplifier UI on the LCD at login
Exec=/bin/sh -lc 'sleep 8; exec /usr/bin/chromium --user-data-dir=${INSTALL_HOME}/.config/chromium-kiosk --password-store=basic --no-first-run --no-default-browser-check --kiosk --start-fullscreen --window-size=1280,800 --ozone-platform=wayland --enable-features=UseOzonePlatform --app=http://127.0.0.1:3000 --noerrdialogs --disable-session-crashed-bubble --disable-infobars --check-for-update-interval=31536000'
Terminal=false
X-GNOME-Autostart-enabled=true
StartupNotify=false
Hidden=false
EOF
}

install_lcd_boot_config() {
  local config_file="/boot/firmware/config.txt"
  sudo test -f "${config_file}" || die "Boot config not found: ${config_file}"
  upsert_root_setting "${config_file}" "display_auto_detect" "0"
  upsert_root_setting "${config_file}" "max_framebuffers" "2"
  ensure_root_line "${config_file}" "dtoverlay=vc4-kms-v3d"
  ensure_root_line "${config_file}" "dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch"
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

log "Installing LCD kiosk session config..."
install_labwc_autostart
install_kanshi_layout
install_kiosk_autostart
log "LCD session config OK: labwc autostart, kanshi layout, and Chromium kiosk autostart installed"

log "Installing Raspberry Pi LCD boot config..."
install_lcd_boot_config
log "Boot config OK: /boot/firmware/config.txt"

log "Installing Hamlib..."
"${REPO_ROOT}/scripts/install-hamlib.sh"
if ! rigctl -V >/dev/null 2>&1; then
  die "Hamlib validation failed: rigctl not available."
fi
log "Hamlib OK: $(rigctl -V | head -n1)"

log "Installing Rust toolchain..."
"${REPO_ROOT}/scripts/install-rust.sh"
source "${INSTALL_HOME}/.cargo/env"
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
if [[ "${REBOOT_REQUIRED}" -eq 1 ]]; then
  log "Reboot required: boot display configuration changed."
fi
