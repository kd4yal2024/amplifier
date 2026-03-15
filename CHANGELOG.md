# Changelog

## 2026-03-14

### Unreleased local changes - Restore Tony-style power sequencing and band-follow behavior

### Runtime behavior

- Restored the intended Tony-style power-button behavior:
  `Blwr` and `Oper` remain single-stage outputs, while `Fil` and `HV` are treated as two-stage sequences
- Fixed live power-state reporting so `Fil` and `HV` expose both stages separately and `Oper` is tracked again in the UI state path
- Fixed the 40m/80m band-output mapping for the live hardware wiring
- Changed TCI and CAT follow logic so a band change received while Tune/Ind/Load are moving is queued and applied after motion finishes instead of being dropped
- Added the live Goodix `0014` touchscreen mapping to the `labwc` setup path so the DSI LCD receives touch input correctly
- Prevented queued TCI/CAT band changes from firing after the operator disables Follow Me or CAT
- Replaced multipart form parsing panics with explicit error handling in the config, selector, load, and power-control paths
- Added operator-visible I2C fault reporting and recovery logging for power-button pin reads and meter reads
- Normalized stepper `max` handling during profile load/save and live encoder control so Tune/Ind/Load continue to work even if a saved position or stored band memory exceeds the previously saved max
- Narrowed I2C warning status updates so background meter faults no longer overwrite unrelated operator actions like save/store/select confirmations
- Added idle watchdogs for both TCI and CAT follow paths so stale "connected" sessions are only tolerated while those services are enabled and still receiving valid frequency updates
- Made the TCI and CAT watchdog timers configurable from the config page and persisted them in profile save/load
- Added explicit `Start` / `Stop` actions for both TCI and CAT on the config page
- Enforced single-service follow behavior so enabling or starting CAT turns TCI off, and enabling or starting TCI turns CAT off
- Reworked the config page into grouped sections for hardware setup, calibration, profiles, and radio follow, replacing the old assignment table with separate axis cards
- Added LCD-specific config-page spacing and viewport containment so the touchscreen layout uses a tighter presentation than the desktop browser view

### Documentation

- Documented the restored power-control behavior and queued band-follow behavior in the README
- Documented the DSI LCD / Goodix touchscreen setup, touch-mapping troubleshooting steps, and installer behavior in the README
- Documented the stepper max normalization and I2C status-message behavior in the README
- Documented the new TCI/CAT follow watchdog behavior in the README
- Documented the new config-page workflow, explicit follow-service controls, and installer provisioning expectations in the README

### Install and desktop setup

- Updated `install-amplifier-controls.sh` to ensure `~/.config/labwc/rc.xml` contains the Goodix `0014` touch mappings for `DSI-1` / `DSI-2`
- Added a best-effort `labwc --reconfigure` step during install so touchscreen mapping changes apply without waiting for a reboot
- Changed the installers to resolve a single `INSTALL_USER` and use that same account for service units, labwc config writes, and compositor reloads

### Commit `8a39414` - Move installer entrypoints to repo root

- Added root-level `install-amplifier-controls.sh` as the primary installer entrypoint for the current checkout
- Added root-level `uninstall-amplifier-controls.sh`
- Converted `scripts/install-amplifier-controls.sh` into a compatibility wrapper that delegates to the root installer
- Updated the README to document repo-root install and uninstall usage

### Commit `d221190` - Harden install flow and document production setup

Production-readiness hardening pass.

### Runtime

- Replaced the HTTP bind `unwrap()` with a clean error path in `src/main.rs`
- Added `AMPLIFIER_BIND` support, defaulting to `0.0.0.0:3000`
- Kept the current TCI/CAT-capable architecture while improving operator-visible status handling

### Profile and UI behavior

- Made config-page profile loads report explicit success or failure
- Updated default-profile actions to produce deterministic status messages
- Sorted profile file lists before rendering the config UI
- Added regression coverage for `static/test.json` so the live profile remains loadable

### Code quality

- Cleaned up Rust warnings across `src/main.rs` and `src/lib.rs`
- Reached a clean `cargo clippy --all-targets --all-features -- -D warnings`
- Kept `cargo test --quiet` passing

### Install and service scripts

- Hardened `scripts/install-amplifier-controls.sh` to avoid forcing an existing checkout onto a different branch
- Added a dirty-worktree preflight check to `scripts/install-amplifier-controls.sh` with a `--force` override
- Escaped generated systemd paths in `scripts/install-amplifier-controls.sh`
- Added post-install service and port verification to `scripts/install-amplifier-controls.sh`
- Hardened `scripts/install-tci-follow-service.sh` so `--url` must include a value
- Ensured `scripts/install-tci-follow-service.sh` always writes `TCI_URL` to the env file
- Removed env-file sourcing from TCI validation flow
- Added more defensive systemd behavior for the TCI follow service

### Notes

- `static/test.json` retains the known-good hardware values from the live system
- A running `target/debug/amplifier` process on port `3000` will still prevent a second instance from starting
