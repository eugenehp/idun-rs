//! Binary decoders for Guardian BLE notification payloads.
//!
//! The Guardian earbud sends EEG/IMU data on the vendor characteristic
//! [`EEG_IMU_CHARACTERISTIC`](crate::protocol::EEG_IMU_CHARACTERISTIC) (`fcc4`).
//! Each BLE notification is a binary packet:
//!
//! ```text
//! byte[0]     : packet type / header tag
//! byte[1]     : sequence index (0–255, wraps)
//! bytes[2..]  : packed measurement samples
//! ```
//!
//! # Decoding strategies
//!
//! | Function | Feature | Description |
//! |---|---|---|
//! | [`parse_eeg_packet`] | always | Extract header (tag + index) and payload |
//! | [`parse_impedance`] | always | Decode impedance from LE unsigned int |
//! | [`try_decode_eeg_12bit`] | `local-decode` | Speculative 12-bit packed EEG decoder |
//! | [`try_decode_imu_i16le`] | `local-decode` | Decode 6×i16 LE as accel + gyro |
//! | [`parse_notification`] | `local-decode` | Heuristic classifier for EEG vs IMU |
//! | [`compute_rms`] | `local-decode` | RMS amplitude for signal quality |
//!
//! # ⚠️ Experimental decoders
//!
//! The Guardian's wire format is proprietary and undocumented. The local
//! decoders use heuristics and speculative formats. For authoritative decoding,
//! use the IDUN Cloud API via [`CloudDecoder`](crate::cloud::CloudDecoder).

#[cfg(feature = "local-decode")]
use crate::types::XyzSample;

/// Parsed EEG/IMU BLE packet header.
///
/// Extracted by [`parse_eeg_packet`] from the first 2 bytes of a notification.
///
/// # Example
///
/// ```rust
/// # use idun::parse::parse_eeg_packet;
/// let data = vec![0xAA, 42, 1, 2, 3, 4, 5];
/// let header = parse_eeg_packet(&data).unwrap();
/// assert_eq!(header.tag, 0xAA);
/// assert_eq!(header.index, 42);
/// assert_eq!(header.payload, vec![1, 2, 3, 4, 5]);
/// ```
#[derive(Debug, Clone)]
pub struct PacketHeader {
    /// Header tag / packet type byte.
    ///
    /// Likely differentiates EEG packets from IMU packets, though the
    /// exact semantics are undocumented.
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
    /// EEG samples decoded from the payload (µV).
    Eeg {
        /// Packet sequence index.
        index: u8,
        /// Decoded voltage samples in µV.
        samples: Vec<f64>,
    },
    /// Accelerometer reading (x, y, z in g).
    Accelerometer {
        /// Packet sequence index.
        index: u8,
        /// 3-axis sample in g.
        sample: XyzSample,
    },
    /// Gyroscope reading (x, y, z in °/s).
    Gyroscope {
        /// Packet sequence index.
        index: u8,
        /// 3-axis sample in °/s.
        sample: XyzSample,
    },
    /// Unknown packet type — could not be classified.
    Unknown {
        /// Packet sequence index.
        index: u8,
        /// Header tag byte.
        tag: u8,
        /// Raw payload bytes.
        payload: Vec<u8>,
    },
}

/// Parse the header from a raw Guardian EEG/IMU BLE notification.
///
/// Returns `None` if the packet is too short (< 2 bytes).
///
/// # Example
///
/// ```rust
/// # use idun::parse::parse_eeg_packet;
/// // Minimum valid packet: just header, no payload
/// let header = parse_eeg_packet(&[0x00, 0xFF]).unwrap();
/// assert_eq!(header.index, 255);
/// assert!(header.payload.is_empty());
///
/// // Too short
/// assert!(parse_eeg_packet(&[0x01]).is_none());
/// ```
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
/// # Heuristics
///
/// - Payloads ≥ 18 bytes: attempt 12-bit EEG decoding
/// - Payloads ≥ 12 bytes: attempt IMU extraction from last 12 bytes
/// - Otherwise: return [`ParsedPacket::Unknown`]
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

    // If payload contains at least 12 bytes, try IMU from the last 12
    if payload.len() >= 12 {
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
/// The earbud sends impedance as a little-endian unsigned integer
/// on characteristic [`IMPEDANCE_CHARACTERISTIC`](crate::protocol::IMPEDANCE_CHARACTERISTIC).
/// The payload length varies (1–4 bytes).
///
/// Returns `None` for empty payloads.
///
/// # Example
///
/// ```rust
/// # use idun::parse::parse_impedance;
/// assert_eq!(parse_impedance(&[0x88, 0x13, 0x00, 0x00]), Some(5000));
/// assert_eq!(parse_impedance(&[200]), Some(200));
/// assert!(parse_impedance(&[]).is_none());
/// ```
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
/// **EXPERIMENTAL**: Assumes 12-bit big-endian packed layout where every
/// 3 bytes encode 2 samples:
///
/// ```text
/// sample₀ = (byte[0] << 4) | (byte[1] >> 4)
/// sample₁ = ((byte[1] & 0x0F) << 8) | byte[2]
/// µV      = 0.48828125 × (raw₁₂ − 2048)
/// ```
///
/// Mid-scale offset is 2048 (12-bit midpoint). Scale factor 0.48828125 µV/LSB
/// yields a range of approximately ±1000 µV.
///
/// Partial groups (< 3 remaining bytes) are silently ignored.
///
/// Requires the `local-decode` feature.
///
/// # Example
///
/// ```rust
/// # #[cfg(feature = "local-decode")]
/// # {
/// # use idun::parse::try_decode_eeg_12bit;
/// // Mid-scale for both samples → 0.0 µV
/// let samples = try_decode_eeg_12bit(&[0x80, 0x08, 0x00]);
/// assert_eq!(samples.len(), 2);
/// assert!((samples[0] - 0.0).abs() < 0.001);
/// # }
/// ```
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
/// Expects at least 12 bytes arranged as:
///
/// ```text
/// [ax_lo, ax_hi, ay_lo, ay_hi, az_lo, az_hi,
///  gx_lo, gx_hi, gy_lo, gy_hi, gz_lo, gz_hi]
/// ```
///
/// Returns `(accelerometer, gyroscope)` as [`XyzSample`] pairs.
///
/// # Scale factors
///
/// Assumes common MEMS IMU configuration (e.g. LSM6DS3):
/// - **Accelerometer**: ±2g range → 0.0000610352 g/LSB (2/32768)
/// - **Gyroscope**: ±245 dps range → 0.0074768 °/s/LSB (245/32768)
///
/// Returns `None` if `data.len() < 12`.
///
/// Requires the `local-decode` feature.
///
/// # Example
///
/// ```rust
/// # #[cfg(feature = "local-decode")]
/// # {
/// # use idun::parse::try_decode_imu_i16le;
/// let data = [0u8; 12]; // all zeros
/// let (accel, gyro) = try_decode_imu_i16le(&data).unwrap();
/// assert_eq!(accel.x, 0.0);
/// assert_eq!(gyro.z, 0.0);
/// # }
/// ```
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

/// Compute the RMS (root mean square) amplitude of EEG samples.
///
/// Useful as a simple signal quality metric. Typical EEG RMS values:
///
/// | Condition | RMS range |
/// |---|---|
/// | Eyes closed (alpha) | 10–30 µV |
/// | Eyes open (beta) | 5–15 µV |
/// | Movement artifact | 50–500 µV |
/// | Blink artifact | 100–300 µV |
///
/// Returns `0.0` for empty input.
///
/// Requires the `local-decode` feature.
///
/// # Example
///
/// ```rust
/// # #[cfg(feature = "local-decode")]
/// # {
/// # use idun::parse::compute_rms;
/// assert!((compute_rms(&[10.0, 10.0, 10.0]) - 10.0).abs() < 0.001);
/// assert_eq!(compute_rms(&[]), 0.0);
/// # }
/// ```
#[cfg(feature = "local-decode")]
pub fn compute_rms(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&v| v * v).sum();
    (sum_sq / samples.len() as f64).sqrt()
}
