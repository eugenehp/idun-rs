# Changelog

## v0.0.1 — 2026-03-17

First release of `idun-rs` — an async Rust library, CLI, and terminal UI for
streaming real-time EEG, IMU, and impedance data from IDUN Guardian earbuds
over BLE.

### Features

- **BLE scanning and connection** — automatic discovery of Guardian earbuds
  (prefix `IGE`), with support for connecting to a specific device by address
- **EEG streaming** — subscribe to EEG/IMU notifications with timestamp
  reconstruction from the 8-bit packet index
- **Impedance streaming** — real-time electrode impedance measurement with
  configurable notch filter (50/60 Hz)
- **Battery monitoring** — read battery level at connect time and polled every 60s
- **Device info** — read MAC address, firmware version, hardware version
- **LED control** — turn the earbud LED on/off during long recordings
- **Cloud decoder** — IDUN Cloud WebSocket client for authoritative EEG decoding
  (requires `IDUN_API_TOKEN`)
- **Experimental EEG decoding** — speculative 12-bit packed sample decoder
  for local waveform visualization (feature = `local-decode`)
- **Experimental IMU decoding** — i16 LE accelerometer + gyroscope extraction

### Binaries

- **`idun-rs`** (CLI) — headless streaming with `--impedance`, `--60hz`,
  `--address`, `--csv`, `--decode`, `--cloud`, `--token` flags
- **`idun-tui`** (TUI) — full-screen ratatui terminal UI with:
  - EEG / IMU / impedance / all-in-one views
  - Device picker overlay with multi-device support
  - Smooth overlay and adjustable Y scale
  - `--simulate` mode (feature = `simulate`) for development without hardware
  - Auto-reconnect on disconnect

### Testing

- 39 tests: parse (20), protocol (7), types (10), doc-tests (2)
