//! Binary decoders for Guardian BLE notification payloads.
//!
//! # Packet format (reverse-engineered from the Python SDK)
//!
//! The Guardian earbud sends EEG/IMU data on the vendor characteristic
//! `beffd56c-…-fcc4`. Each BLE notification is a binary packet:
//!
//! ```text
//! byte[0]     : packet type / header tag
//! byte[1]     : sequence index (0–255, wraps)
//! bytes[2..]  : packed measurement samples
//! ```
//!
//! The header tag (byte\[0\]) likely differentiates EEG packets from IMU
//! packets. The Python SDK does not decode locally — it base64-encodes
//! the entire packet and forwards it to the IDUN cloud.
//!
//! ## Speculative packet type tags
//!
//! Based on common BLE EEG+IMU multiplexing schemes, the header byte
//! may distinguish packet types. We attempt to detect this by examining
//! payload length and tag values.

#[cfg(feature = "local-decode")]
use crate::types::XyzSample;

/// Parsed EEG/IMU BLE packet header.
#[derive(Debug, Clone)]
pub struct PacketHeader {
    /// Header tag / packet type byte.
    pub tag: u8,
    /// Sequence index (0–255, wraps around).
    pub index: u8,
    /// Payload bytes after the 2-byte header.
    pub payload: Vec<u8>,
}

/// Parsed content of a Guardian BLE notification.
///
/// Since EEG and IMU are multiplexed on one characteristic, parsing
/// a single notification may yield EEG data, IMU data, or both.
///
/// Requires the `local-decode` feature.
#[cfg(feature = "local-decode")]
#[derive(Debug, Clone)]
pub enum ParsedPacket {
    /// EEG samples decoded from the payload.
    Eeg {
        index: u8,
        samples: Vec<f64>,
    },
    /// Accelerometer reading (x, y, z in g).
    Accelerometer {
        index: u8,
        sample: XyzSample,
    },
    /// Gyroscope reading (x, y, z in °/s).
    Gyroscope {
        index: u8,
        sample: XyzSample,
    },
    /// Unknown packet type — could not be classified.
    Unknown {
        index: u8,
        tag: u8,
        payload: Vec<u8>,
    },
}

/// Parse the header from a raw Guardian EEG/IMU BLE notification.
///
/// Returns `None` if the packet is too short (< 2 bytes).
pub fn parse_eeg_packet(data: &[u8]) -> Option<PacketHeader> {
    if data.len() < 2 {
        return None;
    }
    Some(PacketHeader {
        tag: data[0],
        index: data[1],
        payload: data[2..].to_vec(),
    })
}

/// Attempt to fully parse a Guardian BLE notification into typed data.
///
/// **EXPERIMENTAL**: The Guardian's wire format is proprietary. This parser
/// uses heuristics on the header tag byte and payload length to classify
/// packets as EEG or IMU data.
///
/// Returns a list of [`ParsedPacket`] variants extracted from the notification.
/// A single notification may produce multiple results if the format packs
/// both EEG and IMU into one packet.
///
/// Requires the `local-decode` feature.
#[cfg(feature = "local-decode")]
pub fn parse_notification(data: &[u8]) -> Vec<ParsedPacket> {
    if data.len() < 2 {
        return vec![];
    }

    let tag = data[0];
    let index = data[1];
    let payload = &data[2..];
    let mut results = Vec::new();

    // Heuristic: try to classify based on tag and payload length.
    //
    // Common patterns in BLE EEG+IMU devices:
    // - EEG packets have longer payloads (20 samples × 12-bit = 30 bytes,
    //   or 20 samples × 16-bit = 40 bytes)
    // - IMU packets have shorter payloads (6 × i16 = 12 bytes for
    //   accel[3] + gyro[3])
    //
    // Without official docs, we try multiple interpretations:

    // If payload is large enough for EEG (≥ 18 bytes for 12 12-bit samples)
    if payload.len() >= 18 {
        let eeg_samples = try_decode_eeg_12bit(payload);
        if !eeg_samples.is_empty() {
            results.push(ParsedPacket::Eeg {
                index,
                samples: eeg_samples,
            });
        }
    }

    // If payload contains exactly 12 bytes → could be accel[3] + gyro[3] as i16 LE
    // Or if the tag suggests IMU data
    if payload.len() >= 12 {
        // Try to extract IMU from the last 12 bytes of the payload,
        // or from a dedicated IMU section.
        // Many devices pack IMU as 6 × i16 (little-endian):
        //   [ax_lo, ax_hi, ay_lo, ay_hi, az_lo, az_hi,
        //    gx_lo, gx_hi, gy_lo, gy_hi, gz_lo, gz_hi]
        if let Some((accel, gyro)) = try_decode_imu_i16le(&payload[payload.len() - 12..]) {
            results.push(ParsedPacket::Accelerometer {
                index,
                sample: accel,
            });
            results.push(ParsedPacket::Gyroscope {
                index,
                sample: gyro,
            });
        }
    }

    // If nothing was decoded, return Unknown
    if results.is_empty() {
        results.push(ParsedPacket::Unknown {
            index,
            tag,
            payload: payload.to_vec(),
        });
    }

    results
}

/// Parse an impedance BLE notification into ohms.
///
/// The earbud sends impedance as a little-endian unsigned integer.
pub fn parse_impedance(data: &[u8]) -> Option<u32> {
    match data.len() {
        0 => None,
        1 => Some(data[0] as u32),
        2 => Some(u16::from_le_bytes([data[0], data[1]]) as u32),
        3 => Some(u32::from_le_bytes([data[0], data[1], data[2], 0])),
        _ => Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]])),
    }
}

/// Attempt to decode EEG samples from the payload using a speculative
/// 12-bit packed unsigned format.
///
/// **EXPERIMENTAL**: Assumes 12-bit big-endian packed layout (3 bytes → 2 samples)
/// with mid-scale at 2048 and scale factor 0.48828125 µV/LSB.
///
/// Requires the `local-decode` feature.
#[cfg(feature = "local-decode")]
pub fn try_decode_eeg_12bit(payload: &[u8]) -> Vec<f64> {
    let mut samples = Vec::with_capacity(payload.len() * 2 / 3);
    let mut i = 0;
    while i + 2 < payload.len() {
        let a = payload[i] as u16;
        let b = payload[i + 1] as u16;
        let c = payload[i + 2] as u16;
        let s0 = (a << 4) | (b >> 4);
        let s1 = ((b & 0x0F) << 8) | c;
        samples.push(0.48828125 * (s0 as f64 - 2048.0));
        samples.push(0.48828125 * (s1 as f64 - 2048.0));
        i += 3;
    }
    samples
}

/// Attempt to decode 6 × i16 LE values as accelerometer + gyroscope.
///
/// Returns `(accel, gyro)` where accel is in g and gyro is in °/s.
///
/// Scale factors (common for MEMS IMUs like LSM6DS3):
/// - Accelerometer: ±2g range → 0.0000610352 g/LSB (2/32768)
/// - Gyroscope: ±245 dps range → 0.0074768 °/s/LSB (245/32768)
///
/// Requires the `local-decode` feature.
#[cfg(feature = "local-decode")]
pub fn try_decode_imu_i16le(data: &[u8]) -> Option<(XyzSample, XyzSample)> {
    if data.len() < 12 {
        return None;
    }

    let read_i16 = |off: usize| -> i16 {
        i16::from_le_bytes([data[off], data[off + 1]])
    };

    const ACCEL_SCALE: f32 = 0.0000610352; // g/LSB
    const GYRO_SCALE: f32 = 0.0074768; // °/s/LSB

    let accel = XyzSample {
        x: read_i16(0) as f32 * ACCEL_SCALE,
        y: read_i16(2) as f32 * ACCEL_SCALE,
        z: read_i16(4) as f32 * ACCEL_SCALE,
    };

    let gyro = XyzSample {
        x: read_i16(6) as f32 * GYRO_SCALE,
        y: read_i16(8) as f32 * GYRO_SCALE,
        z: read_i16(10) as f32 * GYRO_SCALE,
    };

    Some((accel, gyro))
}

/// Compute a simple signal quality metric from a set of EEG samples.
///
/// Returns the RMS (root mean square) amplitude in µV.
///
/// Requires the `local-decode` feature.
#[cfg(feature = "local-decode")]
pub fn compute_rms(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&v| v * v).sum();
    (sum_sq / samples.len() as f64).sqrt()
}
