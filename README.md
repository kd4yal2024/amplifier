# HF Amp Automation

Raspberry Pi controller and web UI for the amplifier/tuner project. The application drives GPIO and MCP23017-backed hardware, serves the local control UI, stores/load profiles from `static/*.json`, and can follow band changes from TCI or CAT.

## Current behavior

- Rust backend with GPIO, I2C, stepper, relay, and profile-management logic
- Web UI served by Axum on `0.0.0.0:3000` by default
- Profile load/save and default-profile selection from the config page
- Optional TCI follow client and CAT follow task
- Installer scripts for the main controller and TCI follow service

## Project layout

```text
/home/pi/github/amplifier
├── Cargo.toml
├── README.md
├── scripts/
├── src/
├── static/
├── templates/
└── tci-client/
```

## Build and run

Install the usual Rust and Debian build requirements first.

```bash
cargo build
cargo run
```

The HTTP listener defaults to `0.0.0.0:3000`.

To bind a different address or port:

```bash
AMPLIFIER_BIND=0.0.0.0:3001 cargo run
```

If startup fails with `Address already in use`, another `amplifier` process is already listening on that port.

## Runtime notes

- Static assets are served from the repo `assets/` directory resolved from `CARGO_MANIFEST_DIR`
- Profiles are loaded from `static/*.json`
- The app now reports bind failures cleanly instead of panicking on `TcpListener::bind`
- TCI and CAT background tasks are spawned during startup; a port conflict can still occur after those tasks begin

### Power controls

- `Blwr` is a direct single-stage output
- `Oper` is a direct single-stage output: `ON` means Operate, `OFF` means Standby
- `Fil` is a two-stage sequence:
  `ON` first energizes stage 1, then energizes stage 2 after the frontend's 3-second delay
  `OFF` drops the staged outputs back down
- `HV` follows the same two-stage pattern as `Fil`
- The live power-state feedback path reports `Fil` and `HV` as two separate stages so the UI can distinguish "step start" from fully on

### Band follow behavior

- TCI and CAT band detection use the latest requested band, even if a new request arrives while Tune/Ind/Load are still moving
- If a band change arrives during motion, it is queued and applied automatically when the motors become idle
- The live system wiring maps the 40m and 80m band outputs in reverse order relative to the last two software slots, and the runtime mapping compensates for that
- TCI Follow Me now has an idle watchdog: while TCI follow is enabled and CAT is not active, the app expects valid TCI frequency frames at least every 15 seconds and forces a reconnect if the websocket goes stale while still marked connected
- CAT auto-band now has a matching watchdog: while CAT is enabled, the app expects valid frequency polls at least every 15 seconds and marks CAT stale if polling stops yielding usable frequency data

### Stepper and status behavior

- Stepper `max` values are normalized during profile load/save so the active position and stored band memories cannot exceed the runtime travel limit used by the encoder loop
- This prevents Tune/Ind/Load selection from appearing dead after loading a profile with stale `pos > max` data
- I2C hardware faults are logged and can still surface as warnings, but they no longer overwrite unrelated operator status messages such as save/store confirmations unless the status bar is already showing an I2C warning

### LCD and touchscreen setup

- The production UI is expected to run on the Raspberry Pi DSI panel as output `DSI-1`
- The touchscreen controller currently appears as `10-0014 Goodix Capacitive TouchScreen` on this hardware
- `labwc` must map that Goodix device to `DSI-1`, otherwise touch can appear dead or land on the wrong screen coordinates while a mouse still works
- The main installer now ensures these `Goodix ... 0014` touch mappings exist in `~/.config/labwc/rc.xml` and tries to reload `labwc`
- If touch stops working after a desktop reset, verify the device name with `libinput list-devices` and confirm the matching `<touch ... mapToOutput="DSI-1" />` entry exists in `~/.config/labwc/rc.xml`

## Installer scripts

### `install-amplifier-controls.sh`

- Runs from the current repo checkout at `/home/pi/github/amplifier`
- Preserves an existing checkout branch instead of forcibly switching it
- Refuses to reinstall over a dirty git worktree unless `--force` is passed
- Installs services for `INSTALL_USER`, defaulting to `${SUDO_USER}` when present or `pi` otherwise
- Installs the release binary to `/usr/local/bin/amplifier`
- Ensures that install user's `labwc` config contains the Goodix touchscreen-to-`DSI-1` mapping needed for touch input on the LCD
- Verifies that `amplifier.service` reaches `active` state and that the HTTP port is actually listening

The old path `scripts/install-amplifier-controls.sh` now delegates to the root installer for backward compatibility.

### `uninstall-amplifier-controls.sh`

- Stops and disables `amplifier.service` and `amplifier-tci-follow.service`
- Removes the installed binaries from `/usr/local/bin`
- Removes the installed systemd unit files
- Keeps `/etc/amplifier/tci-follow.env` unless `--remove-tci-env` is passed

### `scripts/install-tci-follow-service.sh`

- Validates that `--url` is passed with a value
- Uses the same `INSTALL_USER` convention as the main installer for the systemd service account
- Ensures `TCI_URL` is written even if the env file did not already contain that key
- Avoids sourcing the env file during validation
- Adds more defensive systemd behavior to reduce restart loops on static misconfiguration

## Profile handling

- Config-page profile loads now report explicit success or failure
- Setting a default profile updates visible status text instead of failing silently
- Profile file lists are sorted deterministically in the UI
- `static/test.json` is covered by a regression test to confirm it remains loadable and structurally valid

## Verification

These commands were used as the production-readiness baseline:

```bash
bash -n install-amplifier-controls.sh
bash -n scripts/install-tci-follow-service.sh
cargo build
cargo test --quiet
cargo clippy --all-targets --all-features -- -D warnings
```

## Troubleshooting

### Port already in use

Find the listener:

```bash
ss -ltnp | grep ':3000\b'
```

Stop the stale dev process or run on another port:

```bash
pkill -f 'target/debug/amplifier'
AMPLIFIER_BIND=0.0.0.0:3001 cargo run
```

### Touchscreen not activating buttons

First confirm the active touch device name:

```bash
libinput list-devices
```

The expected live device on the DSI panel is currently `10-0014 Goodix Capacitive TouchScreen`.

Then check the `labwc` mapping:

```bash
grep -n "Goodix Capacitive TouchScreen" ~/.config/labwc/rc.xml
```

If the `0014` entries are missing, rerun the installer or add the `mapToOutput="DSI-1"` entry and reload `labwc`:

```bash
LABWC_PID="$(pgrep -u pi -x labwc | head -n1)" labwc --reconfigure
```

### Reinstall safety check

The installer now stops if the target repo has uncommitted changes.

Use the safe path first:

```bash
git -C /path/to/amplifier status --short
```

If you intentionally want to reinstall over a dirty checkout:

```bash
./install-amplifier-controls.sh --force
```

To target a non-default desktop/service account:

```bash
INSTALL_USER=pi ./install-amplifier-controls.sh
```

### Uninstall

```bash
./uninstall-amplifier-controls.sh
```

To also remove the saved TCI env file:

```bash
./uninstall-amplifier-controls.sh --remove-tci-env
```

### TCI follow service status

```bash
systemctl status amplifier-tci-follow --no-pager
journalctl -u amplifier-tci-follow -f
```

### Main app warnings and lint

```bash
cargo clippy --all-targets --all-features -- -D warnings
```
