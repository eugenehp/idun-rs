//! # idun-rs
//!
//! Async Rust library and terminal UI for streaming EEG, IMU, and impedance
//! data from [IDUN Guardian](https://idun.tech/) earbuds over Bluetooth Low
//! Energy.
//!
//! ## Supported hardware
//!
//! | Model | BLE name | EEG | Impedance | IMU | Notes |
//! |---|---|---|---|---|---|
//! | Guardian Earbud 2.1a | `IGEB` | ✓ | ✓ | ✓ | Original model |
//! | Guardian Earbud 3.0a | `IGE-XXXXXX` | ✓ | ✓ | ✓ | Newer hardware |
//!
//! ## IDUN Cloud API token (optional)
//!
//! The Guardian's BLE wire format is proprietary. For authoritative EEG
//! decoding you can use the IDUN Cloud WebSocket API as a fallback:
//!
//! 1. Get an API token from <https://idun.tech/>
//! 2. Export it: `export IDUN_API_TOKEN="your_token"`
//! 3. Pass `--cloud` to the CLI or use [`cloud::CloudDecoder`] in code
//!
//! Local experimental decoding (`--decode` / `local-decode` feature) works
//! without any API token but may not be fully accurate.
//!
//! ## Quick start
//!
//! ```no_run
//! use idun_rs::prelude::*;
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
//! ## Module overview
//!
//! | Module | Purpose |
//! |---|---|
//! | [`prelude`] | One-line glob import of the most commonly needed types |
//! | [`guardian_client`] | BLE scanning, connecting, and the [`GuardianHandle`](guardian_client::GuardianHandle) command API |
//! | [`types`] | All event and data types ([`GuardianEvent`](types::GuardianEvent), [`EegReading`](types::EegReading), etc.) |
//! | [`protocol`] | GATT UUIDs, commands, config bytes, sampling constants |
//! | [`parse`] | Binary decoders for EEG, IMU, and impedance packets |
//! | [`cloud`] | IDUN Cloud WebSocket client for server-side EEG decoding (requires `IDUN_API_TOKEN`) |

pub mod cloud;
pub mod guardian_client;
pub mod parse;
pub mod protocol;
pub mod types;

/// Convenience re-exports for downstream crates.
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
