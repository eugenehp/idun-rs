//! Data types for events emitted by the IDUN Guardian BLE client.
//!
//! All data from the Guardian earbud is delivered as [`GuardianEvent`] variants
//! through the [`tokio::sync::mpsc::Receiver`] returned by
//! [`GuardianClient::connect`](crate::guardian_client::GuardianClient::connect).
//!
//! # Event flow
//!
//! ```text
//! GuardianClient::connect()
//!     → mpsc::Receiver<GuardianEvent>
//!         → Connected(name)
//!         → DeviceInfo { mac, fw, hw }
//!         → Battery { level }
//!         → Eeg { index, timestamp, raw_data, samples, decode_source }  (every ~80ms)
//!         → Accelerometer { index, timestamp, sample }                  (~52 Hz)
//!         → Gyroscope { index, timestamp, sample }                      (~52 Hz)
//!         → Impedance { ohms, kohms, timestamp }                        (when streaming)
//!         → Disconnected
//! ```

/// A single EEG data packet from the Guardian earbud.
///
/// The Guardian is a single-earbud device with a **bipolar montage**: an
/// in-ear-canal electrode (signal) and an outer-ear electrode (reference),
/// both on the same earbud. This yields one EEG channel measuring the
/// voltage difference between the two electrodes. All EEG + IMU data is
/// multiplexed onto a single BLE characteristic.
///
/// Each packet contains up to 20 samples at 250 Hz (80 ms of data).
///
/// # Decoding
///
/// Decoded samples may come from:
/// - **Local decoder** (`local-decode` feature): experimental 12-bit packed format
/// - **Cloud decoder** ([`CloudDecoder`](crate::cloud::CloudDecoder)): IDUN Cloud WebSocket API
/// - **None**: raw bytes only (no decoding attempted)
///
/// Check [`decode_source`](EegReading::decode_source) to determine provenance.
///
/// # Example
///
/// ```rust
/// # use idun::types::{EegReading, DecodeSource};
/// let reading = EegReading {
///     index: 42,
///     timestamp: 1710000000000.0,
///     raw_data: vec![0xAA, 42, 1, 2, 3],
///     samples: Some(vec![0.5, -0.3, 1.2]),
///     decode_source: DecodeSource::Local,
/// };
/// assert_eq!(reading.samples.as_ref().unwrap().len(), 3);
/// ```
#[derive(Debug, Clone)]
pub struct EegReading {
    /// Packet sequence index (0–255, wraps around).
    ///
    /// The Guardian sends a monotonically increasing 8-bit index with each
    /// packet. Used for timestamp reconstruction and packet loss detection.
    pub index: u8,

    /// Wall-clock timestamp in milliseconds since Unix epoch for the first
    /// sample in this packet.
    ///
    /// Reconstructed from the packet index and sample rate, anchored to
    /// the system clock at the time of the first received packet.
    pub timestamp: f64,

    /// Raw binary payload of the BLE notification.
    ///
    /// This is the complete notification value including the 2-byte header
    /// (tag + index) and the packed measurement data. Can be base64-encoded
    /// for cloud upload.
    pub raw_data: Vec<u8>,

    /// Decoded EEG voltage samples in µV, if decoding succeeded.
    ///
    /// - `Some(samples)` — local or cloud decoding produced valid samples.
    ///   Typically 20 samples per packet at 250 Hz.
    /// - `None` — no decoding was attempted or decoding failed.
    pub samples: Option<Vec<f64>>,

    /// Where the decoded samples came from.
    pub decode_source: DecodeSource,
}

/// Indicates how the EEG samples were decoded.
///
/// # Decoding priority
///
/// When both `--decode` and `--cloud` are enabled, the CLI uses this priority:
/// 1. [`Local`](DecodeSource::Local) — experimental 12-bit decoder (instant, no network)
/// 2. [`Cloud`](DecodeSource::Cloud) — IDUN Cloud WebSocket API (authoritative, requires token)
/// 3. [`None`](DecodeSource::None) — raw packet passthrough (always available)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeSource {
    /// No decoding was performed; only raw data is available.
    None,
    /// Decoded locally using the experimental 12-bit decoder.
    ///
    /// ⚠️ The local decoder is speculative — the Guardian's wire format is
    /// proprietary and undocumented. Results may not be accurate.
    Local,
    /// Decoded by the IDUN Cloud API (authoritative).
    ///
    /// Requires an IDUN API token. See [`CloudDecoder`](crate::cloud::CloudDecoder).
    Cloud,
}

/// A single 3-axis inertial measurement sample.
///
/// Used for both accelerometer (units: g) and gyroscope (units: °/s) readings.
///
/// # Example
///
/// ```rust
/// # use idun::types::XyzSample;
/// let gravity = XyzSample { x: 0.0, y: 0.0, z: -1.0 };
/// assert_eq!(gravity.z, -1.0);
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct XyzSample {
    /// X-axis value.
    pub x: f32,
    /// Y-axis value.
    pub y: f32,
    /// Z-axis value.
    pub z: f32,
}

/// Accelerometer reading from the Guardian earbud's IMU.
///
/// The Guardian's EEG+IMU characteristic multiplexes inertial data
/// alongside EEG. Accelerometer values are in **g** (1 g ≈ 9.81 m/s²).
///
/// Scale assumes ±2g range with 0.0000610352 g/LSB (common for LSM6DS3-class
/// MEMS IMUs).
///
/// # Example
///
/// ```rust
/// # use idun::types::{AccelerometerReading, XyzSample};
/// let reading = AccelerometerReading {
///     index: 10,
///     timestamp: 1000.0,
///     sample: XyzSample { x: 0.01, y: -0.02, z: -0.98 },
/// };
/// // z ≈ -1g indicates the earbud is roughly upright
/// assert!(reading.sample.z < -0.9);
/// ```
#[derive(Debug, Clone)]
pub struct AccelerometerReading {
    /// Packet sequence index (0–255).
    pub index: u8,
    /// Wall-clock timestamp in milliseconds since Unix epoch.
    pub timestamp: f64,
    /// Accelerometer sample (x, y, z) in g.
    pub sample: XyzSample,
}

/// Gyroscope reading from the Guardian earbud's IMU.
///
/// Gyroscope values are in **degrees per second** (°/s).
/// Scale assumes ±245 dps range with 0.0074768 °/s/LSB.
///
/// # Example
///
/// ```rust
/// # use idun::types::{GyroscopeReading, XyzSample};
/// let reading = GyroscopeReading {
///     index: 10,
///     timestamp: 1000.0,
///     sample: XyzSample { x: 1.5, y: -0.3, z: 0.0 },
/// };
/// assert_eq!(reading.sample.x, 1.5);
/// ```
#[derive(Debug, Clone)]
pub struct GyroscopeReading {
    /// Packet sequence index (0–255).
    pub index: u8,
    /// Wall-clock timestamp in milliseconds since Unix epoch.
    pub timestamp: f64,
    /// Gyroscope sample (x, y, z) in °/s.
    pub sample: XyzSample,
}

/// Impedance measurement from the Guardian earbud.
///
/// Impedance is measured in ohms and indicates electrode-skin contact quality.
/// Lower values indicate better contact. Typical ranges:
///
/// | Quality | Range |
/// |---|---|
/// | Excellent | < 5 kΩ |
/// | Good | 5–10 kΩ |
/// | Acceptable | 10–20 kΩ |
/// | Poor | > 20 kΩ |
///
/// # Example
///
/// ```rust
/// # use idun::types::ImpedanceReading;
/// let reading = ImpedanceReading {
///     impedance_ohms: 5000,
///     impedance_kohms: 5.0,
///     timestamp: 1710000000000.0,
/// };
/// assert_eq!(reading.impedance_kohms, reading.impedance_ohms as f64 / 1000.0);
/// ```
#[derive(Debug, Clone)]
pub struct ImpedanceReading {
    /// Impedance value in ohms.
    pub impedance_ohms: u32,
    /// Impedance value in kilo-ohms (convenience field: `impedance_ohms / 1000.0`).
    pub impedance_kohms: f64,
    /// Wall-clock timestamp in milliseconds since Unix epoch.
    pub timestamp: f64,
}

/// Battery level reading.
///
/// The Guardian reports battery as a percentage (0–100) via the standard
/// BLE Battery Level characteristic (UUID `0x2A19`). Polled every 60 seconds
/// and also read once at connection time.
///
/// # Example
///
/// ```rust
/// # use idun::types::BatteryReading;
/// let battery = BatteryReading { level: 87 };
/// assert!(battery.level <= 100);
/// ```
#[derive(Debug, Clone)]
pub struct BatteryReading {
    /// Battery state-of-charge in percent (0–100).
    pub level: u8,
}

/// Device information retrieved from the Guardian earbud at connection time.
///
/// Read from standard BLE Device Information Service characteristics:
/// - MAC address: UUID `0x2A25` (Serial Number)
/// - Firmware: UUID `0x2A26` (Firmware Revision)
/// - Hardware: UUID `0x2A27` (Hardware Revision)
///
/// # Example
///
/// ```rust
/// # use idun::types::DeviceInfo;
/// let info = DeviceInfo {
///     mac_address: "AA-BB-CC-DD-EE-FF".into(),
///     firmware_version: "1.2.3".into(),
///     hardware_version: "3.0a".into(),
/// };
/// assert!(info.mac_address.contains('-'));
/// ```
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// MAC address / device ID (format: `"AA-BB-CC-DD-EE-FF"`).
    pub mac_address: String,
    /// Firmware version string (e.g. `"1.2.3"`).
    pub firmware_version: String,
    /// Hardware version string (e.g. `"3.0a"`).
    pub hardware_version: String,
}

/// All data events emitted by the Guardian BLE client.
///
/// Consumers receive these values through the [`mpsc::Receiver`](tokio::sync::mpsc::Receiver)
/// returned by [`GuardianClient::connect`](crate::guardian_client::GuardianClient::connect)
/// or [`GuardianClient::connect_to`](crate::guardian_client::GuardianClient::connect_to).
///
/// # Event order
///
/// After a successful connection, events arrive in this typical order:
/// 1. `Connected(device_name)` — always first
/// 2. `DeviceInfo { mac, firmware, hardware }` — immediately after
/// 3. `Battery { level }` — initial reading
/// 4. `Eeg` / `Accelerometer` / `Gyroscope` — streaming data (after `start_recording()`)
/// 5. `Impedance` — streaming data (after `start_impedance()`)
/// 6. `Disconnected` — BLE link lost
///
/// # Example
///
/// ```no_run
/// # use idun::types::GuardianEvent;
/// # async fn example(mut rx: tokio::sync::mpsc::Receiver<GuardianEvent>) {
/// while let Some(event) = rx.recv().await {
///     match event {
///         GuardianEvent::Eeg(r) => println!("EEG #{}", r.index),
///         GuardianEvent::Battery(b) => println!("Battery: {}%", b.level),
///         GuardianEvent::Disconnected => break,
///         _ => {}
///     }
/// }
/// # }
/// ```
#[derive(Debug, Clone)]
pub enum GuardianEvent {
    /// BLE connection established. Inner string is the advertised device name
    /// (e.g. `"IGEB"` or `"IGE-ABCDEF"`).
    Connected(String),

    /// BLE link was lost (earbud turned off, out of range, etc.).
    ///
    /// After receiving this event, the channel will close. Create a new
    /// connection via [`GuardianClient::connect`](crate::guardian_client::GuardianClient::connect).
    Disconnected,

    /// An EEG data packet from the earbud (~12.5 packets/s at 250 Hz).
    Eeg(EegReading),

    /// Accelerometer data from the earbud's IMU (~52 Hz).
    Accelerometer(AccelerometerReading),

    /// Gyroscope data from the earbud's IMU (~52 Hz).
    Gyroscope(GyroscopeReading),

    /// An impedance measurement (active only during impedance streaming).
    Impedance(ImpedanceReading),

    /// Battery level update (polled every 60 seconds + once at connect).
    Battery(BatteryReading),

    /// Device information (emitted once, immediately after connection).
    DeviceInfo(DeviceInfo),
}
