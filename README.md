# idun

A Rust library, CLI, and terminal UI for streaming real-time EEG, IMU, and
impedance data from **IDUN Guardian** earbuds over Bluetooth Low Energy.

Includes an experimental local packet decoder and an optional cloud fallback
that sends raw BLE packets to the IDUN Cloud API for authoritative decoding.

## All-in-one view (EEG + Accel + Gyro + Impedance)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ IDUN Guardian │ ● IGEB [90ABCDEF] │ ALL │ Bat 87% │ Z:5.3kΩ │ 12.5pkt/s   │
├──────────────────────────────────────────────────────────────────────────────┤
│ EEG  min:-38 max:+42 rms:18 µV  ±500µV                                     │
│ ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿  bipolar channel, rolling 4-second window                │
├──────────────────────────────────────────────────────────────────────────────┤
│ Accel  x:+0.010g  y:+0.020g  z:-0.980g                                     │
│ ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿  red=X  green=Y  blue=Z                                     │
├──────────────────────────────────────────────────────────────────────────────┤
│ Gyro  x:+0.1°/s  y:+0.3°/s  z:-0.0°/s                                     │
│ ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿  red=X  green=Y  blue=Z                                     │
├──────────────────────────────────────────────────────────────────────────────┤
│ Impedance  5.32 kΩ  (120 pts)                                              │
│ ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿                                                          │
├──────────────────────────────────────────────────────────────────────────────┤
│ [Tab]Pick [1]EEG [2]IMU [3]Imp [4]All [+/-]Scale [a]Auto [v]Smooth [q]Quit │
│ MAC SIM-00-11…  FW sim-1.0.0  HW sim-3.0a  A:+0.01,+0.02,-0.98g           │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## Table of contents

- [Supported hardware](#supported-hardware)
- [Installation](#installation)
- [Quick start (library)](#quick-start-library)
- [CLI binary](#cli-binary)
- [Terminal UI](#terminal-ui)
- [Cloud fallback decoding](#cloud-fallback-decoding)
- [BLE protocol reference](#ble-protocol-reference)
- [Architecture](#architecture)
- [Project layout](#project-layout)
- [Configuration](#configuration)
- [Dependencies](#dependencies)
- [Testing](#testing)
- [Comparison with the Python SDK](#comparison-with-the-python-sdk)
- [References](#references)
- [License](#license)

---

## Supported hardware

| Model | BLE name | EEG | Impedance | IMU | Battery |
|---|---|---|---|---|---|
| Guardian Earbud 2.1a | `IGEB` | ✓ | ✓ | ✓ | ✓ |
| Guardian Earbud 3.0a | `IGE-XXXXXX` | ✓ | ✓ | ✓ | ✓ |

### Electrode configuration

The Guardian is a **single earbud** (not a pair) with a **bipolar electrode
montage**:

| Electrode | Location | Role |
|---|---|---|
| In-ear-canal | Inside ear canal | Signal |
| Outer-ear | Concha / outer ear | Reference |

This produces **one EEG channel** measuring the voltage difference between the
two electrodes. EEG and IMU data are multiplexed onto a single BLE GATT
characteristic.

### Data channels

| Channel | Count | Unit | Rate | Source |
|---|---|---|---|---|
| EEG | 1 | µV | 250 Hz | BLE characteristic `fcc4` |
| Accelerometer | 3 (X, Y, Z) | g | ~52 Hz | BLE characteristic `fcc4` (multiplexed) |
| Gyroscope | 3 (X, Y, Z) | °/s | ~52 Hz | BLE characteristic `fcc4` (multiplexed) |
| Impedance | 1 | Ω / kΩ | on-demand | BLE characteristic `fcc8` |
| Battery | 1 | % | polled 60s | Standard BLE `0x2A19` |

---

## API credentials

### IDUN Cloud API token (optional)

The Guardian's BLE wire format is proprietary. For authoritative EEG
decoding, the IDUN Cloud WebSocket API can decode raw BLE packets server-side.

**You do NOT need a token** for raw BLE streaming, local experimental
decoding (`--decode`), the TUI, or impedance/battery reading.
The cloud API token is only needed for authoritative sample-level EEG decoding.

#### Getting a token

1. Create an account at [idun.tech](https://idun.tech/)
2. Navigate to your dashboard and generate an API token
3. Copy the token string (it looks like a long alphanumeric key)

#### Using the token

There are three ways to provide your IDUN API token:

**Option 1: Environment variable (recommended)**

```bash
# Add to your shell profile (~/.bashrc, ~/.zshrc, etc.)
export IDUN_API_TOKEN="your_api_token_here"

# Then use --cloud in CLI or TUI
cargo run --bin idun -- --decode --cloud
```

**Option 2: Command-line flag**

```bash
cargo run --bin idun -- --decode --cloud --token "your_api_token_here"
```

**Option 3: Programmatic (library usage)**

```rust
use idun::cloud::CloudDecoder;

// Direct token
let mut decoder = CloudDecoder::new(
    "your_api_token_here".to_string(),
    "AA-BB-CC-DD-EE-FF".to_string(), // device MAC address
);

// Or from IDUN_API_TOKEN environment variable
let mut decoder = CloudDecoder::from_env(
    "AA-BB-CC-DD-EE-FF".to_string(),
).expect("IDUN_API_TOKEN env var must be set");
```

> **⚠️ Security**: Keep your API token private. Do not commit it to version
> control. Use environment variables or a secrets manager in production.
> Add `IDUN_API_TOKEN` to your `.gitignore` if storing in a file.

---

## Installation

### As a library

```toml
# Cargo.toml — full build (includes TUI + local decoder):
idun = "0.0.1"

# Library only — no TUI, but keep local decoder:
idun = { version = "0.0.1", default-features = false, features = ["local-decode"] }

# Minimal — no TUI, no local decoder (cloud-only or raw passthrough):
idun = { version = "0.0.1", default-features = false }
```

### From source

```shell
git clone https://github.com/user/idun
cd idun
cargo build --release
```

### System dependencies

| Platform | Requirement | Install |
|---|---|---|
| Linux (Debian/Ubuntu) | D-Bus dev libs | `sudo apt install libdbus-1-dev pkg-config` |
| Linux (Fedora) | D-Bus dev libs | `sudo dnf install dbus-devel` |
| macOS | Core Bluetooth | Built-in (grant Bluetooth permission on first run) |
| Windows | WinRT Bluetooth | Built-in |

---

## Quick start (library)

```rust
use idun::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = GuardianClient::new(GuardianClientConfig::default());

    // Option A: scan and auto-connect to the first device found
    let (mut rx, handle) = client.connect().await?;

    // Option B: scan all, then pick a device
    // let devices = client.scan_all().await?;
    // let (mut rx, handle) = client.connect_to(devices[0].clone()).await?;

    handle.start_recording().await?;

    while let Some(event) = rx.recv().await {
        match event {
            GuardianEvent::Eeg(r) => {
                println!("EEG idx={:3} ts={:.0}ms  {} bytes",
                    r.index, r.timestamp, r.raw_data.len());
            }
            GuardianEvent::Accelerometer(r) => {
                println!("Accel x={:+.4}g  y={:+.4}g  z={:+.4}g",
                    r.sample.x, r.sample.y, r.sample.z);
            }
            GuardianEvent::Gyroscope(r) => {
                println!("Gyro  x={:+.2}°/s  y={:+.2}°/s  z={:+.2}°/s",
                    r.sample.x, r.sample.y, r.sample.z);
            }
            GuardianEvent::Impedance(r) => {
                println!("Impedance: {:.2} kΩ  ({} Ω)",
                    r.impedance_kohms, r.impedance_ohms);
            }
            GuardianEvent::Battery(b) => {
                println!("Battery: {}%", b.level);
            }
            GuardianEvent::DeviceInfo(info) => {
                println!("Device: MAC={} FW={} HW={}",
                    info.mac_address, info.firmware_version, info.hardware_version);
            }
            GuardianEvent::Connected(name) => {
                println!("Connected to {name}");
            }
            GuardianEvent::Disconnected => {
                println!("Disconnected");
                break;
            }
        }
    }

    handle.stop_recording().await?;
    handle.disconnect().await?;
    Ok(())
}
```

### Event types

| `GuardianEvent` variant | Payload | When |
|---|---|---|
| `Connected(String)` | Device name | BLE link established |
| `Disconnected` | — | BLE link lost |
| `Eeg(EegReading)` | index, timestamp, raw_data, samples, decode_source | Every ~80ms (20 samples @ 250 Hz) |
| `Accelerometer(AccelerometerReading)` | index, timestamp, XyzSample (g) | ~52 Hz (extracted from EEG+IMU packets) |
| `Gyroscope(GyroscopeReading)` | index, timestamp, XyzSample (°/s) | ~52 Hz (extracted from EEG+IMU packets) |
| `Impedance(ImpedanceReading)` | impedance_ohms, impedance_kohms | During impedance streaming |
| `Battery(BatteryReading)` | level (0–100%) | Every 60 seconds |
| `DeviceInfo(DeviceInfo)` | mac_address, firmware_version, hardware_version | Once after connect |

### GuardianHandle commands

```rust
handle.start_recording().await?;   // Send 'M' command → start EEG+IMU streaming
handle.stop_recording().await?;    // Send 'S' command → stop EEG+IMU streaming
handle.start_impedance().await?;   // Send 'Z' command → start impedance
handle.stop_impedance().await?;    // Send 'X' command → stop impedance
handle.led_on().await?;            // Send 'd1' config → LED on
handle.led_off().await?;           // Send 'd0' config → LED off
handle.read_battery().await?;      // Read standard BLE battery characteristic
handle.is_connected().await;       // Check BLE link status
handle.disconnect().await?;        // Graceful BLE disconnect
```

---

## CLI binary

```shell
# Basic EEG streaming
cargo run --bin idun

# Impedance streaming
cargo run --bin idun -- --impedance

# 60 Hz mains notch filter (Americas, Japan)
cargo run --bin idun -- --60hz

# Connect to a specific device by name or address
cargo run --bin idun -- --address "IGE-ABCDEF"

# Record to CSV file
cargo run --bin idun -- --csv recording.csv

# Experimental local EEG decoding + CSV
cargo run --bin idun -- --decode --csv decoded.csv

# Cloud fallback decoding
cargo run --bin idun -- --decode --cloud --token my-api-token

# Custom scan timeout
cargo run --bin idun -- --timeout 30
```

### CLI flags

| Flag | Description | Default |
|---|---|---|
| `--impedance` | Stream impedance instead of EEG | off |
| `--60hz` | Use 60 Hz notch filter | 50 Hz |
| `--address ADDR` | Connect to a specific BLE address or name | auto-connect first |
| `--csv FILE` | Write data to a CSV file | off |
| `--decode` | Attempt experimental 12-bit EEG sample decoding | off |
| `--cloud` | Enable IDUN Cloud fallback when local decoding fails | off |
| `--token TOKEN` | IDUN API token (alternative to `IDUN_API_TOKEN` env var) | env var |
| `--timeout SECS` | BLE scan timeout in seconds | 15 |
| `--help` | Show help | — |

### Interactive commands (type + Enter while streaming)

| Command | Action |
|---|---|
| `q` | Gracefully disconnect and quit |
| `b` | Read battery level |
| `z` | Start impedance streaming (stops EEG) |
| `s` | Stop current measurement |
| `m` | Start EEG/IMU measurement |
| `d` | Disconnect from device |

### Enable verbose logging

```shell
RUST_LOG=debug cargo run --bin idun
RUST_LOG=idun=debug cargo run --bin idun   # library logs only
```

---

## Terminal UI

```shell
# Real device — scan and auto-connect
cargo run --bin idun-tui

# Simulated data (no hardware needed — requires `simulate` feature)
cargo run --bin idun-tui --features simulate -- --simulate

# 60 Hz notch filter
cargo run --bin idun-tui -- --60hz
```

The TUI writes logs to `$TMPDIR/idun-tui.log` (never to stderr, which would
corrupt the alternate-screen display).

### Views

Press `1`–`4` to switch between views:

| Key | View | Charts shown |
|---|---|---|
| `1` | EEG | Single EEG channel (full height, with smooth overlay) |
| `2` | IMU | Accelerometer X/Y/Z + Gyroscope X/Y/Z (stacked) |
| `3` | Impedance | Impedance history (auto-scaled) |
| `4` | **All** | EEG + Accel + Gyro + Impedance (default) |

### EEG view (`1`)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ EEG  min:-38 max:+42 rms:18 µV  ±500µV                                     │
│                                                                              │
│ ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿  bipolar EEG channel, rolling 4-second window             │
│                        smooth overlay (cyan) over raw (dim)                  │
│                                                                              │
│ 0s                                                                    4s     │
└──────────────────────────────────────────────────────────────────────────────┘
```

The EEG chart shows a 4-second rolling window at 250 Hz (1000 points).
**Smooth mode** (on by default, toggle with `v`) draws the raw signal in a dim
colour as background, then overlays a 9-sample moving average (~36ms) in
bright cyan.

### IMU view (`2`)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ Accel  x:+0.010g  y:+0.020g  z:-0.980g                                     │
│ ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿  red=X  green=Y  blue=Z  (auto-scaled Y axis)               │
├──────────────────────────────────────────────────────────────────────────────┤
│ Gyro  x:+0.1°/s  y:+0.3°/s  z:-0.0°/s                                     │
│ ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿  red=X  green=Y  blue=Z  (auto-scaled Y axis)               │
└──────────────────────────────────────────────────────────────────────────────┘
```

Accelerometer and gyroscope each show 3 axes (X=red, Y=green, Z=blue) with
auto-scaling Y axis. Buffer holds ~4 seconds at 52 Hz (208 points per axis).

### Device picker (`Tab`)

```
┌──── ⠋ Scanning…  (2 found) ─────────────────────────────────────────────┐
│  ● IGEB  [90ABCDEF]  ← connected                                        │
│ ▶  IGE-123456  [12345678]                                                │
│                                                                          │
│  [↑↓] Nav  [↵] Connect  [s] Rescan  [Esc] Close                         │
└──────────────────────────────────────────────────────────────────────────┘
```

If the device disconnects unexpectedly, the TUI clears the device list and
starts a fresh BLE scan after a 2-second delay (giving the earbud time to
resume advertising).

### TUI key reference

| Key | Context | Action |
|---|---|---|
| `Tab` | streaming | open device picker |
| `1` | streaming | EEG view (single bipolar channel) |
| `2` | streaming | IMU view (accel + gyro, 6 axes) |
| `3` | streaming | Impedance view |
| `4` | streaming | All-in-one view (EEG + accel + gyro + impedance) |
| `+` / `=` | EEG view | zoom out (increase µV scale) |
| `-` | EEG view | zoom in (decrease µV scale) |
| `a` | EEG view | auto-scale to current peak amplitude |
| `v` | streaming | toggle smooth overlay on/off |
| `p` | streaming | pause — stop measurement |
| `r` | streaming | resume — start measurement |
| `z` | streaming | start impedance streaming (stops EEG) |
| `c` | streaming | clear all waveform buffers |
| `s` | streaming / picker | trigger a fresh BLE scan |
| `d` | streaming | disconnect current device |
| `q` / `Esc` | streaming | quit |
| `↑` / `↓` | picker | navigate device list |
| `Enter` | picker | connect to selected device |
| `Esc` | picker | close picker |
| `Ctrl-C` | anywhere | force quit |

### Y-axis scale presets (EEG)

| Preset | Range |
|---|---|
| ±10 µV | Very sensitive |
| ±25 µV | |
| ±50 µV | Simulator default |
| ±100 µV | |
| ±200 µV | |
| **±500 µV** | **Real device default** |
| ±1000 µV | |
| ±2000 µV | Full range |

### Simulator

The built-in simulator (`--simulate`) generates physiologically realistic
synthetic data without hardware:

**EEG signal components:**

| Component | Frequency | Amplitude | Description |
|---|---|---|---|
| Alpha | 10 Hz | ±20 µV | Dominant resting rhythm |
| Beta | 22 Hz | ±6 µV | Active/alert state |
| Theta | 6 Hz | ±10 µV | Drowsiness / meditation |
| Delta | 2 Hz | ±4 µV | Deep sleep baseline |
| Noise | broadband | ±4 µV | Deterministic pseudo-random |
| Blink | ~0.17 Hz | +150 µV peak | Gaussian artifact every ~6s |
| Jaw clench | ~0.10 Hz | ±80 µV | 65 Hz EMG burst every ~10s |

**IMU simulation:**

| Channel | Pattern | Amplitude |
|---|---|---|
| Accel X | 0.3 Hz sine | ±0.01 g |
| Accel Y | 0.5 Hz cosine | ±0.02 g |
| Accel Z | -1.0 g + 0.1 Hz sine | ±0.005 g (gravity-dominated) |
| Gyro X | 0.2 Hz sine | ±2.5 °/s |
| Gyro Y | 0.3 Hz cosine | ±1.8 °/s |
| Gyro Z | 0.15 Hz sine | ±0.7 °/s |

**Other simulated data:**
- Impedance: 6 kΩ baseline with slow drift + spike artifacts every ~16s
- Battery: drains from 92% at ~1%/min

```shell
cargo run --bin idun-tui --features simulate -- --simulate
```

---

## Cloud fallback decoding

When `--decode` and `--cloud` are both enabled, the CLI uses a three-level
failsafe chain for EEG packet decoding:

```
Raw BLE packet
    │
    ▼
┌─────────────────────┐
│ 1. Local 12-bit     │ ← experimental decoder (instant, no network)
│    decoder           │   validity check: non-empty, not all zeros,
│                      │   RMS between 0.1–2000 µV
└────────┬────────────┘
         │ failed?
         ▼
┌─────────────────────┐
│ 2. IDUN Cloud API   │ ← authoritative decoder (requires API token)
│    WebSocket         │   connects lazily on first local failure
│                      │   sticky failure: disabled for session if fails
└────────┬────────────┘
         │ unavailable?
         ▼
┌─────────────────────┐
│ 3. Raw packet       │ ← base64-encoded raw bytes (always available)
│    passthrough       │
└─────────────────────┘
```

### Cloud API protocol

The IDUN Cloud uses a WebSocket endpoint at `wss://ws-api.idun.cloud` with
query-parameter authentication:

```
wss://ws-api.idun.cloud?authorization={api_token}
```

**Session flow:**

1. Connect WebSocket with auth token
2. Send `startNewRecording` → receive `recordingUpdate` with `recordingId`
3. Send `subscribeLiveStreamInsights` for `RAW_EEG` / `FILTERED_EEG` / `IMU`
4. Send `publishRawMeasurements` with base64-encoded raw BLE packets
5. Receive `liveStreamInsights` with decoded data
6. Send `endOngoingRecording` when done

**Available cloud stream types:**

| Type | Description |
|---|---|
| `RAW_EEG` | Unfiltered EEG samples |
| `FILTERED_EEG` | Bandpass-filtered EEG |
| `IMU` | Accelerometer + gyroscope |

**Available cloud predictions:**

| Type | Description |
|---|---|
| `FFT` | Frequency spectrum |
| `JAW_CLENCH` | Jaw clench detection |
| `BIN_HEOG` | Horizontal eye movement |
| `QUALITY_SCORE` | Signal quality metric |
| `CALM_SCORE` | Relaxation level |
| `COGNITIVE_READINESS` | Cognitive readiness score |

### Usage

```shell
# Cloud fallback with environment variable
export IDUN_API_TOKEN=my-api-token
cargo run --bin idun -- --decode --cloud

# Cloud fallback with inline token
cargo run --bin idun -- --decode --cloud --token my-api-token

# Cloud-only (skip local decoding attempt)
cargo run --bin idun -- --cloud --token my-api-token
```

### Library usage

```rust
use idun::cloud::CloudDecoder;

let mut decoder = CloudDecoder::new(
    "my-api-token".to_string(),
    "AA-BB-CC-DD-EE-FF".to_string(), // device MAC address
);

// Lazy connect — only opens WebSocket when needed
decoder.connect().await?;

// Send a raw BLE packet for cloud decoding
decoder.send_raw_packet(&raw_ble_bytes, timestamp_ms, packet_index).await?;

// Poll for decoded results
if let Some(decoded_json) = decoder.recv_decoded().await? {
    println!("Cloud decoded: {}", decoded_json);
}

// Clean shutdown
decoder.disconnect().await?;
```

---

## BLE protocol reference

### GATT characteristics

| UUID suffix | Full UUID | Purpose | Direction |
|---|---|---|---|
| `fcc4` | `beffd56c-c915-48f5-930d-4c1feee0fcc4` | EEG + IMU data | Notify |
| `fcc8` | `beffd56c-c915-48f5-930d-4c1feee0fcc8` | Impedance data | Notify |
| `fcc9` | `beffd56c-c915-48f5-930d-4c1feee0fcc9` | Configuration | Write |
| `fcca` | `beffd56c-c915-48f5-930d-4c1feee0fcca` | Commands | Write |
| `2A19` | Standard BLE SIG | Battery Level | Read |
| `2A25` | Standard BLE SIG | Serial Number / MAC ID | Read |
| `2A26` | Standard BLE SIG | Firmware Revision | Read |
| `2A27` | Standard BLE SIG | Hardware Revision | Read |

### Commands (written to `fcca`)

| Byte | ASCII | Action |
|---|---|---|
| `0x4D` | `M` | Start EEG/IMU measurement |
| `0x53` | `S` | Stop EEG/IMU measurement |
| `0x5A` | `Z` | Start impedance streaming |
| `0x58` | `X` | Stop impedance streaming |

### Configuration (written to `fcc9`)

| Value | ASCII | Action |
|---|---|---|
| `0x64 0x31` | `d1` | LED on |
| `0x64 0x30` | `d0` | LED off |
| `0x6E 0x30` | `n0` | 50 Hz notch filter (Europe, Asia) |
| `0x6E 0x31` | `n1` | 60 Hz notch filter (Americas, Japan) |

### EEG+IMU packet format

Each BLE notification on `fcc4`:

```
byte[0]    : header tag / packet type
byte[1]    : sequence index (0–255, wraps around)
bytes[2..] : packed measurement samples (EEG and/or IMU)
```

The header byte (byte[0]) likely differentiates EEG packets from IMU packets.
The Python SDK does not decode locally — it base64-encodes the entire packet
and forwards it to the IDUN Cloud for processing.

**Sampling:**
- EEG sample rate: **250 Hz**
- EEG samples per BLE packet: **20**
- Packet index range: **0–255** (8-bit, wraps)
- BLE notification interval: ~80 ms

### Experimental local EEG decoder

The local decoder assumes **12-bit big-endian packed unsigned** format
(3 bytes → 2 samples), similar to the Muse Classic format:

```
sample₀ = (byte[0] << 4) | (byte[1] >> 4)          // even samples
sample₁ = ((byte[1] & 0x0F) << 8) | byte[2]        // odd samples
µV      = 0.48828125 × (sample₁₂ − 2048)           // mid-scale offset
```

**⚠️ This format is speculative.** The actual Guardian wire format is
proprietary and undocumented. The decoder uses a validity heuristic (non-empty,
not all zeros, RMS between 0.1–2000 µV) and falls back to cloud decoding or
raw passthrough when the heuristic rejects the result.

### Experimental IMU decoder

IMU data may be packed as **6 × i16 little-endian** values (accelerometer XYZ
+ gyroscope XYZ) in the last 12 bytes of EEG+IMU packets:

```
accel[axis] = i16_LE × 0.0000610352    // g/LSB  (±2g range)
gyro[axis]  = i16_LE × 0.0074768       // °/s/LSB (±245 dps range)
```

Scale factors assume a common MEMS IMU (e.g., LSM6DS3) configuration.

### Impedance format

Impedance notifications on `fcc8` are little-endian unsigned integers
(1–4 bytes depending on payload size):

```
impedance_ohms = u32_LE(data[0..4])
impedance_kohms = impedance_ohms / 1000.0
```

### Timestamp reconstruction

Since BLE notifications arrive with variable latency, timestamps are
reconstructed from the 8-bit packet index:

1. First packet anchors to `now()`.
2. Subsequent packets compute elapsed time from the index delta:
   `dt = (current_index − last_index) mod 256 × (samples_per_packet / sample_rate)`
3. Wrap-around (255 → 0) is handled automatically.

---

## Architecture

```
User Code → GuardianClient
               ├── scan_all()       → Vec<GuardianDevice>
               ├── connect()        → (Receiver<GuardianEvent>, GuardianHandle)
               └── connect_to(dev)  → (Receiver<GuardianEvent>, GuardianHandle)

                             ┌── GuardianEvent::Eeg
                             ├── GuardianEvent::Accelerometer
  mpsc::Receiver ◄───────── ├── GuardianEvent::Gyroscope
   (async channel)           ├── GuardianEvent::Impedance
                             ├── GuardianEvent::Battery
                             ├── GuardianEvent::DeviceInfo
                             ├── GuardianEvent::Connected
                             └── GuardianEvent::Disconnected

GuardianHandle                          CloudDecoder
    ├── start_recording()                   ├── connect()
    ├── stop_recording()                    ├── send_raw_packet()
    ├── start_impedance()                   ├── try_recv_decoded()
    ├── stop_impedance()                    ├── recv_decoded()
    ├── led_on() / led_off()                ├── is_connected()
    ├── read_battery()                      └── disconnect()
    ├── is_connected()
    └── disconnect()
```

### Internal data flow

```
┌─────────────────┐     BLE notify     ┌──────────────────┐
│  Guardian Earbud │ ──────────────────▶│  btleplug        │
│  (BLE peripheral)│    char fcc4       │  notification    │
└─────────────────┘                    │  stream          │
                                       └────────┬─────────┘
                                                │
                        ┌───────────────────────┼──────────────────────────┐
                        │                       │                          │
                        ▼                       ▼                          ▼
               ┌────────────────┐    ┌────────────────────┐    ┌─────────────────┐
               │ parse_eeg_packet│   │ try_decode_imu_i16le│   │parse_impedance  │
               │ try_decode_12bit│   │ (last 12 bytes)    │   │(char fcc8)      │
               └───────┬────────┘   └─────────┬──────────┘   └───────┬─────────┘
                       │                       │                       │
                       ▼                       ▼                       ▼
            GuardianEvent::Eeg    GuardianEvent::      GuardianEvent::Impedance
                                  Accelerometer /
                                  Gyroscope

                    All events → mpsc::channel → user's Receiver<GuardianEvent>
```

### Background tasks

The client spawns several `tokio` tasks:

| Task | Purpose | Lifecycle |
|---|---|---|
| Notification dispatcher | Routes BLE notifications to appropriate parsers | Connected → Disconnected |
| Battery poller | Reads battery level every 60 seconds | Connected → Disconnected |
| Disconnect watcher | Monitors BLE adapter events for disconnect | Connected → Disconnected |

---

## Project layout

```
idun/
├── Cargo.toml               # Package metadata, features, dependencies
├── Cargo.lock
├── README.md                 # This file
├── CHANGELOG.md              # Version history
├── LICENSE                   # MIT license
├── .gitignore
├── src/
│   ├── lib.rs                # Crate root: module declarations + prelude
│   ├── types.rs              # EegReading, AccelerometerReading, GyroscopeReading,
│   │                         # ImpedanceReading, BatteryReading, DeviceInfo,
│   │                         # XyzSample, DecodeSource, GuardianEvent
│   ├── protocol.rs           # GATT UUIDs, commands, config, sampling constants
│   ├── parse.rs              # Binary decoders: EEG 12-bit, IMU i16 LE, impedance
│   ├── guardian_client.rs    # GuardianClient (scan/connect) + GuardianHandle
│   │                         # BLE notification dispatch, disconnect watcher,
│   │                         # battery polling, timestamp reconstruction
│   ├── cloud.rs              # CloudDecoder: IDUN Cloud WebSocket client
│   │                         # Lazy connect, session management, sticky failure
│   ├── main.rs               # Headless CLI: --decode --cloud --csv --impedance
│   └── bin/
│       └── tui.rs            # Full-screen TUI: EEG + IMU + impedance charts,
│                             # device picker, smooth overlay, --simulate mode
└── tests/
    ├── parse_tests.rs        # 33 tests: EEG decoder, IMU decoder, impedance,
    │                         # RMS, notification parser, edge cases
    ├── protocol_tests.rs     # 10 tests: UUID uniqueness/SIG, command bytes, constants
    ├── types_tests.rs        # 12 tests: all event types, clone, debug, equality
    ├── cloud_tests.rs        # 5 tests: CloudDecoder offline, from_env, try_recv
    ├── client_config_tests.rs # 3 tests: GuardianClientConfig defaults and clone
    └── prelude_tests.rs      # 5 tests: all prelude re-exports
```

---

## Configuration

```rust
let config = GuardianClientConfig {
    mains_freq_60hz: false,       // true for 60 Hz notch (Americas, Japan)
    scan_timeout_secs: 15,        // BLE scan timeout
    name_prefix: "IGE".into(),    // Match "IGEB" and "IGE-XXXXXX" devices
};
let client = GuardianClient::new(config);
```

| Field | Type | Default | Description |
|---|---|---|---|
| `mains_freq_60hz` | `bool` | `false` | `true` → send `n1` (60 Hz notch); `false` → send `n0` (50 Hz) |
| `scan_timeout_secs` | `u64` | `15` | Abort BLE scan after this many seconds |
| `name_prefix` | `String` | `"IGE"` | Only connect to devices whose name starts with this |

---

## Dependencies

| Crate | Version | Purpose |
|---|---|---|
| [btleplug](https://crates.io/crates/btleplug) | 0.11 | Cross-platform BLE (scan, connect, GATT) |
| [tokio](https://tokio.rs) | 1.x | Async runtime (full features) |
| [tokio-tungstenite](https://crates.io/crates/tokio-tungstenite) | 0.24 | WebSocket client for IDUN Cloud API |
| [uuid](https://crates.io/crates/uuid) | 1.x | GATT UUID construction |
| [futures](https://crates.io/crates/futures) | 0.3 | Stream combinators for BLE notifications |
| [serde](https://crates.io/crates/serde) + [serde_json](https://crates.io/crates/serde_json) | 1.x | JSON serialization for cloud protocol |
| [base64](https://crates.io/crates/base64) | 0.22 | Encoding raw BLE packets for cloud upload |
| [log](https://crates.io/crates/log) + [env_logger](https://crates.io/crates/env_logger) | 0.4 / 0.11 | Structured logging |
| [anyhow](https://crates.io/crates/anyhow) | 1.x | Error handling |
| [ratatui](https://ratatui.rs) | 0.30 | Terminal UI framework (optional, `tui` feature) |
| [crossterm](https://crates.io/crates/crossterm) | 0.29 | Terminal backend (optional, `tui` feature) |

### Feature flags

| Feature | Default | Effect |
|---|---|---|
| `tui` | **on** | Enables `ratatui` + `crossterm` deps and the `idun-tui` binary |
| `local-decode` | **on** | Enables experimental local EEG 12-bit decoder, IMU i16 LE decoder, `compute_rms()`, and `parse_notification()`. Without this feature, raw BLE packets are passed through undecoded and `--decode` is ignored with a warning. Cloud fallback still works. |
| `simulate` | off | Enables `--simulate` flag in the TUI for synthetic data generation without hardware |

```shell
# Build without TUI (library + CLI only)
cargo build --no-default-features --features local-decode

# Build without local decoder (cloud-only or raw passthrough)
cargo build --no-default-features --features tui

# Build with everything (default)
cargo build

# Minimal build (no TUI, no local decoder — just BLE + cloud)
cargo build --no-default-features
```

---

## Testing

```shell
# Run all 70 tests
cargo test

# Run with verbose output
cargo test -- --nocapture

# Run specific test suite
cargo test --test parse_tests
cargo test --test protocol_tests
cargo test --test types_tests
```

### Test coverage

| Suite | Tests | Covers | Feature-gated |
|---|---|---|---|
| `parse_tests` | 12 | Header parser, impedance parser (1/2/3/4+ bytes), edge cases | — |
| `parse_tests::local_decode` | 21 | EEG 12-bit decoder (min/mid/max), IMU i16 LE (pos/neg/exact/extra), RMS, notification parser | `local-decode` |
| `protocol_tests` | 10 | UUID uniqueness/SIG compliance, command/config bytes, sampling constants, channel name | — |
| `types_tests` | 12 | All event types, Clone/Debug, XyzSample, DecodeSource, EEG with decoded samples | — |
| `cloud_tests` | 5 | CloudDecoder construction, `from_env` (missing/present), `try_recv`, `CloudDecodedEeg` clone | — |
| `client_config_tests` | 3 | Default config, custom config, config Clone | — |
| `prelude_tests` | 5 | All prelude re-exports compile and are accessible | — |
| Doc-tests | 2 | Library root example, CloudDecoder example | — |
| **Total** | **70** | | |

Without the `local-decode` feature, 21 decoder-specific tests are skipped.

---

## Comparison with the Python SDK

| Feature | Python SDK (`idun_guardian_sdk`) | `idun` |
|---|---|---|
| BLE scanning & connection | ✓ (`bleak`) | ✓ (`btleplug`) |
| EEG/IMU streaming (raw) | ✓ | ✓ |
| Impedance streaming | ✓ | ✓ |
| Battery reading | ✓ | ✓ |
| Device info (MAC, FW, HW) | ✓ | ✓ |
| LED control | ✓ | ✓ |
| Notch filter config | ✓ | ✓ |
| Local EEG decoding | ✗ (cloud only) | ✓ (experimental 12-bit) |
| Local IMU decoding | ✗ (cloud only) | ✓ (experimental i16 LE) |
| Cloud WebSocket client | ✓ (full) | ✓ (decoding fallback) |
| Cloud HTTP API | ✓ (recordings, reports) | ✗ |
| Cloud predictions (FFT, jaw clench, etc.) | ✓ | ✗ |
| LSL output (`pylsl`) | ✓ | ✗ |
| Real-time TUI visualization | ✗ | ✓ (ratatui) |
| Multi-channel waveform display | ✗ | ✓ (EEG + Accel XYZ + Gyro XYZ + Impedance) |
| Simulated data mode | ✗ | ✓ (physiologically realistic) |
| Language | Python 3 | Rust |
| Async runtime | `asyncio` | `tokio` |
| Cross-platform | Linux, macOS, Windows | Linux, macOS, Windows |

### What this crate does NOT include

- **Cloud HTTP API** — recording management, file downloads, reports
- **Cloud predictions** — FFT, jaw clench, BIN_HEOG, calm score, cognitive readiness
- **LSL streaming** — Lab Streaming Layer output for integration with other tools
- **Recording storage** — cloud-side recording persistence and retrieval

These features require IDUN API credentials and are best handled by the
official Python SDK or a separate Rust HTTP client.

---

## Troubleshooting

### No devices found during scan

1. Ensure the Guardian earbud is **powered on** and **not connected** to another device
2. Check that Bluetooth is enabled on your system
3. On Linux: ensure `bluez` is running (`sudo systemctl status bluetooth`)
4. Try increasing the scan timeout: `--timeout 30`
5. Verify the earbud name starts with `IGEB` or `IGE-`

### Permission denied (Linux)

```shell
# Add your user to the bluetooth group
sudo usermod -aG bluetooth $USER
# Or run with sudo (not recommended for production)
sudo cargo run --bin idun
```

### macOS Bluetooth permission

macOS requires Bluetooth permission for each application. On first run:
1. A system dialog appears: _"idun" would like to use Bluetooth_
2. Click **Allow**
3. If previously denied: **System Settings → Privacy & Security → Bluetooth**

### EEG samples are all `None`

The experimental local decoder may not match the Guardian's actual wire format.
Options:
1. Use `--cloud --token <TOKEN>` for authoritative cloud decoding
2. Use `--decode` to see if the local decoder produces plausible values
3. Check `RUST_LOG=debug` output for packet hex dumps

### Connection drops frequently

- Move closer to the earbud (BLE range is typically 10m)
- Ensure no other BLE clients are connected to the same earbud
- The TUI automatically reconnects after a 2-second delay

---

## References

- [IDUN Technologies](https://idun.tech/) — Official IDUN Guardian manufacturer
- [IDUN Guardian SDK](https://github.com/iduntechnologies/idun-guardian-sdk) — Official Python SDK (BLE + cloud API)
- [btleplug](https://github.com/deviceplug/btleplug) — Cross-platform BLE library for Rust
- [ratatui](https://ratatui.rs/) — Terminal UI framework for Rust
- [muse-rs](https://github.com/eugenehp/muse-rs) — Similar Rust project for Muse EEG headsets (architectural reference)

---

## Citation

If you use this software in academic work, please cite it:

```bibtex
@software{hauptmann2026idun,
  author       = {Hauptmann, Eugene},
  title        = {idun: Async Rust Client for IDUN Guardian EEG Earbuds over BLE},
  year         = {2026},
  url          = {https://github.com/eugenehp/idun},
  version      = {0.0.1},
  license      = {MIT}
}
```

## License

[MIT](./LICENSE)
