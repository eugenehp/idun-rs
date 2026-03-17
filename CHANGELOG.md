# Changelog

## v0.0.3 — 2026-03-17

### Added

- `GuardianClientConfig::api_token` — optional field to supply the IDUN Cloud API
  token directly without relying on the `IDUN_API_TOKEN` environment variable
- `CloudDecoder::from_config` — creates a `CloudDecoder` from a
  `GuardianClientConfig`, preferring the inline token and falling back to the
  env var when the field is `None`
- New tests in `client_config_tests.rs`: `api_token_default_is_none`,
  extended `custom_config` asserting roundtrip of the `api_token` field

---

## v0.0.2 — 2026-03-17

### Changed

- **Renamed crate** from `idun-rs` / `idun_rs` to `idun` across all source,
  tests, documentation, and Cargo metadata
- **Comprehensive doc comments** on all public types, functions, modules, and
  constants with examples, tables, and cross-references
- **Updated README** with detailed IDUN Cloud API token documentation
  (3 methods: env var, CLI flag, programmatic), expanded test coverage table,
  updated project layout

### Added

- `tests/cloud_tests.rs` — 5 tests for `CloudDecoder` offline behaviour,
  `from_env` (missing/present), `try_recv_decoded`, `CloudDecodedEeg` clone
- `tests/client_config_tests.rs` — 3 tests for `GuardianClientConfig`
  defaults, custom values, and Clone
- `tests/prelude_tests.rs` — 5 tests verifying all prelude re-exports
- Extended `parse_tests.rs` with 13 new tests: 3-byte/max/extra impedance,
  max index, large payload, EEG min/max 12-bit values, negative IMU,
  exact/extra IMU bytes, single/negative RMS, unknown short notification
- Extended `protocol_tests.rs` with 3 new tests: `DEVICE_INFO_SERVICE` SIG
  UUID, `BATTERY_CHARACTERISTIC` SIG UUID, `EEG_CHANNEL_NAME`
- Extended `types_tests.rs` with 2 new tests: `EegReading` with decoded
  samples + `DecodeSource::Local`, cloud source inequality

### Testing

- **70 tests** total (up from 39): parse (33), protocol (10), types (12),
  cloud (5), client config (3), prelude (5), doc-tests (2)

---

## v0.0.1 — 2026-03-17

First release of `idun` — an async Rust library, CLI, and terminal UI for
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

- **`idun`** (CLI) — headless streaming with `--impedance`, `--60hz`,
  `--address`, `--csv`, `--decode`, `--cloud`, `--token` flags
- **`idun-tui`** (TUI) — full-screen ratatui terminal UI with:
  - EEG / IMU / impedance / all-in-one views
  - Device picker overlay with multi-device support
  - Smooth overlay and adjustable Y scale
  - `--simulate` mode (feature = `simulate`) for development without hardware
  - Auto-reconnect on disconnect

### Testing

- 39 tests: parse (20), protocol (7), types (10), doc-tests (2)
