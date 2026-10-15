//! GATT UUIDs, sampling constants, and BLE wire-format helpers for IDUN Guardian earbuds.
//!
//! The Guardian uses standard Bluetooth SIG UUIDs for Device Information
//! (service `0x180A`) and Battery (characteristic `0x2A19`), plus vendor-specific
//! UUIDs for EEG/IMU data, impedance, configuration, and command channels.
//!
//! # GATT characteristic map
//!
//! | UUID suffix | Full UUID | Purpose | Direction |
//! |---|---|---|---|
//! | `fcc3` | `beffd56c-c915-48f5-930d-4c1feee0fcc3` | Guardian vendor service | — |
//! | `fcc4` | `beffd56c-c915-48f5-930d-4c1feee0fcc4` | EEG + IMU data | Notify |
//! | `fcc8` | `beffd56c-c915-48f5-930d-4c1feee0fcc8` | Impedance data | Notify |
//! | `fcc9` | `beffd56c-c915-48f5-930d-4c1feee0fcc9` | Configuration | Write |
//! | `fcca` | `beffd56c-c915-48f5-930d-4c1feee0fcca` | Commands | Write |
//! | `fccb` | `beffd56c-c915-48f5-930d-4c1feee0fccb` | Device ID (MAC) | Read |
//! | `2A19` | Standard BLE SIG | Battery Level | Read |
//! | `2A25` | Standard BLE SIG | Serial Number / MAC ID | Read (blocklisted) |
//! | `2A26` | Standard BLE SIG | Firmware Revision | Read |
//! | `2A27` | Standard BLE SIG | Hardware Revision | Read |
//!
//! # Commands
//!
//! Written to the command characteristic (`fcca`):
//!
//! | Byte | ASCII | Action |
//! |---|---|---|
//! | `0x4D` | `M` | Start EEG/IMU measurement |
//! | `0x53` | `S` | Stop EEG/IMU measurement |
//! | `0x5A` | `Z` | Start impedance streaming |
//! | `0x58` | `X` | Stop impedance streaming |
//!
//! # Configuration
//!
//! Written to the configuration characteristic (`fcc9`):
//!
//! | Value | ASCII | Action |
//! |---|---|---|
//! | `0x64 0x31` | `d1` | LED on |
//! | `0x64 0x30` | `d0` | LED off |
//! | `0x6E 0x30` | `n0` | 50 Hz notch filter (Europe, Asia) |
//! | `0x6E 0x31` | `n1` | 60 Hz notch filter (Americas, Japan) |

use uuid::Uuid;

// ── Standard Bluetooth SIG UUIDs ─────────────────────────────────────────────

/// Device Information Service UUID (`0x180A`, standard BLE SIG).
pub const DEVICE_INFO_SERVICE: Uuid =
    Uuid::from_u128(0x0000180a_0000_1000_8000_00805f9b34fb);

/// Battery Service UUID (`0x180F`, standard BLE SIG).
///
/// Holds [`BATTERY_CHARACTERISTIC`].
pub const BATTERY_SERVICE: Uuid =
    Uuid::from_u128(0x0000180f_0000_1000_8000_00805f9b34fb);

/// Serial Number / MAC ID characteristic (`0x2A25`, standard BLE SIG).
///
/// The Guardian stores its MAC address here as a UTF-8 string
/// (e.g. `"AA:BB:CC:DD:EE:FF"`), but the Web Bluetooth GATT blocklist excludes
/// this characteristic as a tracking identifier, so it is never readable. The
/// client reads [`DEVICE_ID_CHARACTERISTIC`] instead.
pub const MAC_ID_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0x00002a25_0000_1000_8000_00805f9b34fb);

/// Firmware Revision characteristic (`0x2A26`, standard BLE SIG).
pub const FIRMWARE_VERSION_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0x00002a26_0000_1000_8000_00805f9b34fb);

/// Hardware Revision characteristic (`0x2A27`, standard BLE SIG).
pub const HARDWARE_VERSION_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0x00002a27_0000_1000_8000_00805f9b34fb);

/// Battery Level characteristic (`0x2A19`, standard BLE SIG).
///
/// Returns a single byte (0–100) representing battery state-of-charge.
pub const BATTERY_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0x00002a19_0000_1000_8000_00805f9b34fb);

// ── Vendor-specific UUIDs (IDUN Guardian) ────────────────────────────────────
//
// All vendor UUIDs share the base `beffd56c-c915-48f5-930d-4c1feee0fc__`,
// differing only in the last two bytes.

/// Guardian vendor service (`fcc3`).
///
/// Every vendor characteristic below lives under it. Named in the device
/// request — together with [`DEVICE_INFO_SERVICE`] and [`BATTERY_SERVICE`] —
/// because a Web Bluetooth grant covers exactly the services a request lists.
pub const GUARDIAN_SERVICE: Uuid =
    Uuid::from_u128(0xbeffd56c_c915_48f5_930d_4c1feee0fcc3);

/// EEG + IMU measurement characteristic (`fcc4`, notifications).
///
/// The earbud streams raw EEG/IMU data packets on this characteristic.
/// Each notification is a binary packet:
///
/// ```text
/// byte[0]    : header tag / packet type
/// byte[1]    : sequence index (0–255, wraps around)
/// bytes[2..] : packed measurement samples (EEG and/or IMU)
/// ```
///
/// At 250 Hz with 20 samples per packet, notifications arrive every ~80 ms.
pub const EEG_IMU_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0xbeffd56c_c915_48f5_930d_4c1feee0fcc4);

/// Impedance measurement characteristic (`fcc8`, notifications).
///
/// When impedance streaming is active, the earbud sends impedance values
/// (in ohms) as little-endian unsigned integers on this characteristic.
pub const IMPEDANCE_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0xbeffd56c_c915_48f5_930d_4c1feee0fcc8);

/// Configuration characteristic (`fcc9`, write).
///
/// Used to configure LED state and notch filter frequency.
/// See [`CFG_LED_ON`], [`CFG_LED_OFF`], [`CFG_NOTCH_50HZ`], [`CFG_NOTCH_60HZ`].
pub const CONFIG_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0xbeffd56c_c915_48f5_930d_4c1feee0fcc9);

/// Command characteristic (`fcca`, write).
///
/// Used to start/stop measurements.
/// See [`CMD_START_MEASUREMENT`], [`CMD_STOP_MEASUREMENT`],
/// [`CMD_START_IMPEDANCE`], [`CMD_STOP_IMPEDANCE`].
pub const COMMAND_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0xbeffd56c_c915_48f5_930d_4c1feee0fcca);

/// Device ID characteristic (`fccb`, read).
///
/// The earbud's MAC address as a UTF-8 string with a one-character
/// terminator, e.g. `"AA:BB:CC:DD:EE:FF"` plus one byte. The client reads
/// the MAC from here rather than from [`MAC_ID_CHARACTERISTIC`], which is on
/// the Web Bluetooth GATT blocklist.
pub const DEVICE_ID_CHARACTERISTIC: Uuid =
    Uuid::from_u128(0xbeffd56c_c915_48f5_930d_4c1feee0fccb);

// ── Device commands ──────────────────────────────────────────────────────────

/// Command byte to start EEG/IMU measurement (`"M"` = `0x4D`).
pub const CMD_START_MEASUREMENT: &[u8] = b"M";

/// Command byte to stop EEG/IMU measurement (`"S"` = `0x53`).
pub const CMD_STOP_MEASUREMENT: &[u8] = b"S";

/// Command byte to start impedance streaming (`"Z"` = `0x5A`).
pub const CMD_START_IMPEDANCE: &[u8] = b"Z";

/// Command byte to stop impedance streaming (`"X"` = `0x58`).
pub const CMD_STOP_IMPEDANCE: &[u8] = b"X";

// ── Configuration values ─────────────────────────────────────────────────────

/// Turn the LED on (`"d1"` = `0x64 0x31`).
pub const CFG_LED_ON: &[u8] = b"d1";

/// Turn the LED off (`"d0"` = `0x64 0x30`).
pub const CFG_LED_OFF: &[u8] = b"d0";

/// Set notch filter to 50 Hz (`"n0"` — Europe, Asia, Africa, Oceania).
pub const CFG_NOTCH_50HZ: &[u8] = b"n0";

/// Set notch filter to 60 Hz (`"n1"` — Americas, Japan, South Korea, Taiwan).
pub const CFG_NOTCH_60HZ: &[u8] = b"n1";

// ── Sampling constants ───────────────────────────────────────────────────────

/// EEG sample rate in Hz (250 Hz).
pub const EEG_SAMPLE_RATE: f64 = 250.0;

/// Number of EEG samples per BLE notification packet (20).
///
/// At 250 Hz, this yields one packet every 80 ms (~12.5 packets/s).
pub const EEG_SAMPLES_PER_PACKET: usize = 20;

/// Maximum packet index before wrap-around.
///
/// The Guardian uses an 8-bit index (0–255), so this is 256.
pub const MAX_PACKET_INDEX: u16 = 256;

/// Device name prefix used to identify Guardian earbuds during BLE scanning.
///
/// Matches both `"IGEB"` (hardware v2.1a) and `"IGE-XXXXXX"` (hardware v3.0a).
pub const DEVICE_NAME_PREFIX: &str = "IGE";

// ── Channel labels ───────────────────────────────────────────────────────────

/// EEG channel name (`"EEG"`).
///
/// The Guardian uses a bipolar montage with two electrodes on a single earbud:
/// an in-ear-canal electrode (signal) and an outer-ear electrode (reference).
/// This produces one EEG channel.
pub const EEG_CHANNEL_NAME: &str = "EEG";
