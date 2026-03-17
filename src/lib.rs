//! # idun
//!
//! Async Rust library, CLI, and terminal UI for streaming real-time EEG, IMU,
//! and impedance data from [IDUN Guardian](https://idun.tech/) earbuds over
//! Bluetooth Low Energy.
//!
//! ## Overview
//!
//! The IDUN Guardian is a single-earbud EEG device with a bipolar electrode
//! montage (in-ear-canal signal + outer-ear reference), producing one EEG
//! channel at 250 Hz. EEG, accelerometer, and gyroscope data are multiplexed
//! onto a single BLE GATT characteristic. This crate provides:
//!
//! - **BLE scanning and connection** via [`guardian_client::GuardianClient`]
//! - **Real-time event streaming** through an async [`tokio::sync::mpsc`] channel
//! - **Device commands** (start/stop recording, impedance, LED, battery) via [`guardian_client::GuardianHandle`]
//! - **Experimental local decoding** of EEG (12-bit packed) and IMU (i16 LE) packets
//! - **Cloud fallback decoding** via the IDUN WebSocket API ([`cloud::CloudDecoder`])
//!
//! ## IDUN Cloud API token
//!
//! The Guardian's BLE wire format is proprietary. For authoritative EEG decoding
//! you can use the IDUN Cloud WebSocket API as a fallback:
//!
//! 1. Obtain an API token from <https://idun.tech/>
//! 2. Set the environment variable:
//!    ```bash
//!    export IDUN_API_TOKEN="your_api_token"
//!    ```
//! 3. Or pass the token programmatically to [`cloud::CloudDecoder::new`]
//!
//! Local experimental decoding and raw BLE streaming work **without** any API
//! token. The cloud is only needed for authoritative sample-level decoding.
//!
//! ## Quick start
//!
//! ```no_run
//! use idun::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let client = GuardianClient::new(GuardianClientConfig::default());
//!     let (mut rx, handle) = client.connect().await?;
//!     handle.start_recording().await?;
//!
//!     while let Some(event) = rx.recv().await {
//!         match event {
//!             GuardianEvent::Eeg(r) => println!("EEG idx={} ts={:.0}ms", r.index, r.timestamp),
//!             GuardianEvent::Disconnected => break,
//!             _ => {}
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!
//! ## Feature flags
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `tui` | ✓ | Enables `ratatui` + `crossterm` and the `idun-tui` binary |
//! | `local-decode` | ✓ | Enables experimental local EEG/IMU packet decoders |
//! | `simulate` | ✗ | Enables `--simulate` for synthetic data without hardware |
//!
//! ## Modules
//!
//! | Module | Purpose |
//! |---|---|
//! | [`prelude`] | One-line glob import of the most commonly needed types |
//! | [`guardian_client`] | BLE scanning, connecting, and the [`GuardianHandle`](guardian_client::GuardianHandle) command API |
//! | [`types`] | All event and data types ([`GuardianEvent`](types::GuardianEvent), [`EegReading`](types::EegReading), etc.) |
//! | [`protocol`] | GATT UUIDs, commands, config bytes, sampling constants |
//! | [`parse`] | Binary decoders for EEG, IMU, and impedance packets |
//! | [`cloud`] | IDUN Cloud WebSocket client for server-side EEG decoding |

pub mod cloud;
pub mod guardian_client;
pub mod parse;
pub mod protocol;
pub mod types;

/// Convenience re-exports for downstream crates.
///
/// Import everything you need with a single glob:
///
/// ```rust
/// use idun::prelude::*;
/// ```
///
/// This re-exports:
/// - Client types: [`GuardianClient`](crate::guardian_client::GuardianClient),
///   [`GuardianClientConfig`](crate::guardian_client::GuardianClientConfig),
///   [`GuardianDevice`](crate::guardian_client::GuardianDevice),
///   [`GuardianHandle`](crate::guardian_client::GuardianHandle)
/// - Cloud types: [`CloudDecoder`](crate::cloud::CloudDecoder),
///   [`CloudDecodedEeg`](crate::cloud::CloudDecodedEeg)
/// - Event types: all variants of [`GuardianEvent`](crate::types::GuardianEvent)
///   and their payload structs
/// - Constants: [`EEG_SAMPLE_RATE`](crate::protocol::EEG_SAMPLE_RATE),
///   [`EEG_SAMPLES_PER_PACKET`](crate::protocol::EEG_SAMPLES_PER_PACKET),
///   [`EEG_CHANNEL_NAME`](crate::protocol::EEG_CHANNEL_NAME)
/// - Parse functions: [`parse_eeg_packet`](crate::parse::parse_eeg_packet),
///   [`parse_impedance`](crate::parse::parse_impedance)
/// - (With `local-decode` feature): [`try_decode_eeg_12bit`](crate::parse::try_decode_eeg_12bit),
///   [`try_decode_imu_i16le`](crate::parse::try_decode_imu_i16le),
///   [`compute_rms`](crate::parse::compute_rms)
pub mod prelude {
    pub use crate::cloud::{CloudDecoder, CloudDecodedEeg};
    pub use crate::guardian_client::{
        GuardianClient, GuardianClientConfig, GuardianDevice, GuardianHandle,
    };
    pub use crate::types::{
        AccelerometerReading, BatteryReading, DecodeSource, DeviceInfo, EegReading,
        GuardianEvent, GyroscopeReading, ImpedanceReading, XyzSample,
    };
    pub use crate::protocol::{
        EEG_CHANNEL_NAME, EEG_SAMPLE_RATE, EEG_SAMPLES_PER_PACKET,
    };
    pub use crate::parse::{parse_eeg_packet, parse_impedance};
    #[cfg(feature = "local-decode")]
    pub use crate::parse::{
        compute_rms, try_decode_eeg_12bit, try_decode_imu_i16le,
    };
}
