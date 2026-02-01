#!/usr/bin/env bash
set -euo pipefail

# Installs Rust toolchain via rustup + basic build deps

sudo apt-get update -y
sudo apt-get install -y \
  curl ca-certificates git \
  build-essential pkg-config

# Install rustup (official install method)
# rust-lang.org uses rustup and provides the curl|sh command. :contentReference[oaicite:1]{index=1}
if ! command -v rustup >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal
fi

# Ensure current shell has cargo in PATH
# rustup installs into ~/.cargo/bin :contentReference[oaicite:2]{index=2}
source "$HOME/.cargo/env"

# Pick a sane default toolchain
rustup default stable

echo
echo "[OK] rustc: $(rustc -V)"
echo "[OK] cargo: $(cargo -V)"
