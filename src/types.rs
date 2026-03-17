//! Data types for events emitted by the IDUN Guardian BLE client.

/// A single EEG data packet from the Guardian earbud.
///
/// The Guardian is a single-earbud device with a **bipolar montage**: an
/// in-ear-canal electrode (signal) and an outer-ear electrode (reference),
/// both on the same earbud. This yields one EEG channel measuring the
/// voltage difference between the two electrodes. All EEG + IMU data is
/// multiplexed onto a single BLE characteristic.
///
/// Decoded samples may come from the local experimental decoder or from the
/// IDUN Cloud API (when cloud fallback is enabled).
#[derive(Debug, Clone)]
pub struct EegReading {
    /// Packet sequence index (0–255, wraps around).
    pub index: u8,
    /// Wall-clock timestamp in milliseconds since Unix epoch for the first
    /// sample in this packet, reconstructed from the packet index and sample rate.
    pub timestamp: f64,
    /// Raw binary payload of the BLE notification (base64-encodable for cloud upload).
    pub raw_data: Vec<u8>,
    /// Decoded EEG voltage samples in µV, if decoding succeeded.
    ///
    /// - `Some(samples)` — local or cloud decoding produced valid samples.
    /// - `None` — no decoding was attempted or decoding failed.
    pub samples: Option<Vec<f64>>,
    /// Where the decoded samples came from.
    pub decode_source: DecodeSource,
}

/// Indicates how the EEG samples were decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeSource {
    /// No decoding was performed; only raw data is available.
    None,
    /// Decoded locally using the experimental 12-bit decoder.
    Local,
    /// Decoded by the IDUN Cloud API (authoritative).
    Cloud,
}

/// A single 3-axis inertial measurement sample.
#[derive(Debug, Clone, Copy, Default)]
pub struct XyzSample {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Accelerometer reading from the Guardian earbud's IMU.
///
/// The Guardian's EEG+IMU characteristic multiplexes inertial data
/// alongside EEG. Accelerometer values are in g (1 g ≈ 9.81 m/s²).
#[derive(Debug, Clone)]
pub struct AccelerometerReading {
    /// Packet sequence index.
    pub index: u8,
    /// Wall-clock timestamp in milliseconds since Unix epoch.
    pub timestamp: f64,
    /// Accelerometer sample (x, y, z) in g.
    pub sample: XyzSample,
}

/// Gyroscope reading from the Guardian earbud's IMU.
///
/// Gyroscope values are in degrees per second (°/s).
#[derive(Debug, Clone)]
pub struct GyroscopeReading {
    /// Packet sequence index.
    pub index: u8,
    /// Wall-clock timestamp in milliseconds since Unix epoch.
    pub timestamp: f64,
    /// Gyroscope sample (x, y, z) in °/s.
    pub sample: XyzSample,
}

/// Impedance measurement from the Guardian earbud.
///
/// Impedance is measured in ohms and indicates electrode-skin contact quality.
/// Lower values indicate better contact.
#[derive(Debug, Clone)]
pub struct ImpedanceReading {
    /// Impedance value in ohms.
    pub impedance_ohms: u32,
    /// Impedance value in kilo-ohms (convenience).
    pub impedance_kohms: f64,
    /// Wall-clock timestamp in milliseconds since Unix epoch.
    pub timestamp: f64,
}

/// Battery level reading.
#[derive(Debug, Clone)]
pub struct BatteryReading {
    /// Battery state-of-charge in percent (0–100).
    pub level: u8,
}

/// Device information retrieved from the Guardian earbud.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// MAC address / device ID (e.g. "AA-BB-CC-DD-EE-FF").
    pub mac_address: String,
    /// Firmware version string.
    pub firmware_version: String,
    /// Hardware version string.
    pub hardware_version: String,
}

/// All data events emitted by [`crate::guardian_client::GuardianClient`].
///
/// Consumers receive these values through the `mpsc::Receiver` returned by
/// [`crate::guardian_client::GuardianClient::connect`].
#[derive(Debug, Clone)]
pub enum GuardianEvent {
    /// BLE connection established. Inner string is the device name.
    Connected(String),

    /// BLE link was lost (earbud turned off, out of range, etc.).
    Disconnected,

    /// An EEG data packet from the earbud.
    Eeg(EegReading),

    /// Accelerometer data from the earbud's IMU.
    Accelerometer(AccelerometerReading),

    /// Gyroscope data from the earbud's IMU.
    Gyroscope(GyroscopeReading),

    /// An impedance measurement from the earbud.
    Impedance(ImpedanceReading),

    /// Battery level update.
    Battery(BatteryReading),

    /// Device information (MAC, firmware, hardware version).
    DeviceInfo(DeviceInfo),
}
