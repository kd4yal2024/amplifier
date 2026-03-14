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

## Installer scripts

### `install-amplifier-controls.sh`

- Runs from the current repo checkout at `/home/pi/github/amplifier`
- Preserves an existing checkout branch instead of forcibly switching it
- Refuses to reinstall over a dirty git worktree unless `--force` is passed
- Installs the release binary to `/usr/local/bin/amplifier`
- Verifies that `amplifier.service` reaches `active` state and that the HTTP port is actually listening

The old path `scripts/install-amplifier-controls.sh` now delegates to the root installer for backward compatibility.

### `uninstall-amplifier-controls.sh`

- Stops and disables `amplifier.service` and `amplifier-tci-follow.service`
- Removes the installed binaries from `/usr/local/bin`
- Removes the installed systemd unit files
- Keeps `/etc/amplifier/tci-follow.env` unless `--remove-tci-env` is passed

### `scripts/install-tci-follow-service.sh`

- Validates that `--url` is passed with a value
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
