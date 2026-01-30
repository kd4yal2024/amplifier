````text
## HF Amp Automation (Raspberry Pi Controller + Web UI)

### A Raspberry Pi–based controller for an HF amplifier/tuner project, with a local web UI and hardware I/O via GPIO and (optionally) I²C expanders.

This repo is aimed at being **deployable like an appliance**:

- predictable layout
- minimal hard-coded paths
- easy install / run / recover
- logs you can actually read when you’re tired

---

## What it does (today)

- Runs a **Rust backend** that talks to local hardware (GPIO and/or I²C expanders).
- Serves a **web UI** (HTML/JS/CSS) from a static directory.
- Can be run interactively (`cargo run`) or as a **systemd service**.
- Supports ongoing hardware work (encoder, stepper driver, opto modules, etc.) without rewriting the whole stack.

---

## Project layout

> This is the intended structure. If your tree differs slightly, align to this so docs/scripts don’t go stale.

```text
pi@amp:~/github/amplifier
├── Cargo.lock
├── Cargo.toml
├── README.md
├── src
│   ├── lib.rs
│   └── main.rs
├── static
│   ├── amplifier3.js
│   ├── amplifier.css
│   ├── amplifier.json
│   ├── main-polish.css
│   ├── skin.css
│   ├── test.json
│   ├── test123.json
│   ├── test_sse.css
│   └── ui-polish.css
├── templates
│   ├── amplifier2.html
│   ├── amplifier2_old.html
│   ├── config2.html
│   └── test_sse.html
````

---

## Quick start (developer)

### 1) Install build dependencies

On Raspberry Pi OS / Debian-based distros:

```bash
sudo apt update
sudo apt install -y build-essential pkg-config
```

If you use GPIO/I²C libraries that require headers (varies by approach), install what your code needs.

### 2) Build + run

```bash
cargo build
cargo run
```

By default, the server should start and serve the web UI (if `static/` is found).

---

## Configuration (no hard-coded paths)

Hard-coding absolute filesystem paths is a deployment booby trap. Use environment variables (or a config file) instead.

Suggested env vars:

* `AMP_BIND=0.0.0.0:8080` — web bind address
* `AMP_STATIC_DIR=./static` — where the UI assets live
* `AMP_CONFIG=./config/config.yaml` — optional structured config
* `RUST_LOG=info` — logging level (or `debug` when you’re hunting gremlins)

Example:

```bash
export AMP_BIND="0.0.0.0:8080"
export AMP_STATIC_DIR="./static"
export RUST_LOG="info"
cargo run
```

---

## Running as a service (systemd)

### 1) Install unit file

Copy the service template:

```bash
sudo install -D -m 0644 systemd/amplifier.service /etc/systemd/system/amplifier.service
sudo systemctl daemon-reload
```

### 2) Enable + start

```bash
sudo systemctl enable --now amplifier
sudo systemctl status amplifier
```

### 3) Logs

```bash
journalctl -u amplifier -f
```

---

## Hardware notes (what we’ve learned so far)

### Voltage levels matter

* Raspberry Pi GPIO is **3.3V** logic.
* If any peripheral is 5V logic, use proper level shifting / isolation.

### GPIO vs I²C expanders

* Direct GPIO is simplest.
* For lots of inputs/outputs, I²C expanders (e.g., MCP23017) keep wiring sane.
* If using I²C:

  * confirm `i2cdetect -y 1`
  * confirm device address (commonly `0x20` depending on A0/A1/A2)

### Encoders

* Mechanical encoders can bounce; software debouncing helps.
* Keep wiring short, use pull-ups/pull-downs deliberately, and don’t trust “it worked once” as a permanent truth.

### Steppers / drivers (tuner)

* Stepper drivers (e.g., TB6600) and opto-isolation modules can help protect the Pi.
* Keep motor power/noise away from logic wiring. Grounding and routing matter.

---

## Development workflow

### Common commands

```bash
cargo fmt
cargo clippy
cargo test
```

---

## Troubleshooting

* **Web UI not loading / missing assets**
  Check `AMP_STATIC_DIR` and confirm the directory exists on the target system.

* **Works on your Pi but not on Tony’s**
  That’s usually a path/config issue. Kill absolute paths; use env/config.

* **GPIO/I²C not responding**
  Confirm device visibility (`gpiodetect`, `i2cdetect -y 1`) and permissions.
  If running as a service, remember systemd runs with a different environment.

More in: `docs/troubleshooting.md`

---

## Roadmap (near-term)

*TBD*

---

## License

TBD

```
```
