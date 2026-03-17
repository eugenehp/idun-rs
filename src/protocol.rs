//! GATT UUIDs, sampling constants, and BLE wire-format helpers for IDUN Guardian earbuds.
//!
//! The Guardian uses standard Bluetooth SIG UUIDs for Device Information
//! (service 0x180A) and Battery (characteristic 0x2A19), plus vendor-specific
//! UUIDs for EEG/IMU data, impedance, configuration, and command channels.

use uuid::Uuid;

// ── Standard Bluetooth SIG UUIDs ─────────────────────────────────────────────

/// Device Information Service UUID (standard BLE SIG).
pub const DEVICE_INFO_SERVICE: Uuid =
    Uuid::from_u128(0x0000180a_0000_1000_8000_00805f9b34fb);

/// Serial Number / MAC ID characteristic (standard BLE SIG).
pub const MAC_ID_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0x00002a25_0000_1000_8000_00805f9b34fb);

/// Firmware Revision characteristic (standard BLE SIG).
pub const FIRMWARE_VERSION_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0x00002a26_0000_1000_8000_00805f9b34fb);

/// Hardware Revision characteristic (standard BLE SIG).
pub const HARDWARE_VERSION_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0x00002a27_0000_1000_8000_00805f9b34fb);

/// Battery Level characteristic (standard BLE SIG, 0–100%).
pub const BATTERY_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0x00002a19_0000_1000_8000_00805f9b34fb);

// ── Vendor-specific UUIDs (IDUN Guardian) ────────────────────────────────────

/// EEG + IMU measurement characteristic (notifications).
///
/// The earbud streams raw EEG/IMU data packets on this characteristic.
/// Each notification is a binary packet containing a header byte, a sequence
/// index, and packed measurement samples.
pub const EEG_IMU_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0xbeffd56c_c915_48f5_930d_4c1feee0fcc4);

/// Impedance measurement characteristic (notifications).
///
/// When impedance streaming is active, the earbud sends impedance values
/// (in ohms) as little-endian integers on this characteristic.
pub const IMPEDANCE_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0xbeffd56c_c915_48f5_930d_4c1feee0fcc8);

/// Configuration characteristic (write).
///
/// Used to configure LED state and notch filter frequency:
/// - `"d1"` = LED on
/// - `"d0"` = LED off
/// - `"n0"` = 50 Hz notch filter
/// - `"n1"` = 60 Hz notch filter
pub const CONFIG_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0xbeffd56c_c915_48f5_930d_4c1feee0fcc9);

/// Command characteristic (write).
///
/// Used to start/stop measurements:
/// - `"M"` = start EEG/IMU recording
/// - `"S"` = stop EEG/IMU recording
/// - `"Z"` = start impedance streaming
/// - `"X"` = stop impedance streaming
pub const COMMAND_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0xbeffd56c_c915_48f5_930d_4c1feee0fcca);

// ── Device commands ──────────────────────────────────────────────────────────

/// Command byte to start EEG/IMU measurement.
pub const CMD_START_MEASUREMENT: &[u8] = b"M";

/// Command byte to stop EEG/IMU measurement.
pub const CMD_STOP_MEASUREMENT: &[u8] = b"S";

/// Command byte to start impedance streaming.
pub const CMD_START_IMPEDANCE: &[u8] = b"Z";

/// Command byte to stop impedance streaming.
pub const CMD_STOP_IMPEDANCE: &[u8] = b"X";

// ── Configuration values ─────────────────────────────────────────────────────

/// Turn the LED on.
pub const CFG_LED_ON: &[u8] = b"d1";

/// Turn the LED off.
pub const CFG_LED_OFF: &[u8] = b"d0";

/// Set notch filter to 50 Hz (Europe, Asia, etc.).
pub const CFG_NOTCH_50HZ: &[u8] = b"n0";

/// Set notch filter to 60 Hz (Americas, Japan, etc.).
pub const CFG_NOTCH_60HZ: &[u8] = b"n1";

// ── Sampling constants ───────────────────────────────────────────────────────

/// EEG sample rate in Hz.
pub const EEG_SAMPLE_RATE: f64 = 250.0;

/// Number of EEG samples per BLE notification packet.
pub const EEG_SAMPLES_PER_PACKET: usize = 20;

/// Maximum packet index before wrap-around (0–255).
pub const MAX_PACKET_INDEX: u16 = 256;

/// Device name prefix used to identify Guardian earbuds during BLE scanning.
/// Covers both "IGEB" (hardware v2.1a) and "IGE-XXXXXX" (hardware v3.0a).
pub const DEVICE_NAME_PREFIX: &str = "IGE";

// ── Channel labels ───────────────────────────────────────────────────────────

/// EEG channel name.
///
/// The Guardian uses a bipolar montage with two electrodes on a single earbud:
/// an in-ear-canal electrode (signal) and an outer-ear electrode (reference).
/// This produces one EEG channel.
pub const EEG_CHANNEL_NAME: &str = "EEG";
