# Changelog

## 2026-03-14

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

- Added root-level `install-amplifier-controls.sh` for installs from the current checkout
- Added root-level `uninstall-amplifier-controls.sh`
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
