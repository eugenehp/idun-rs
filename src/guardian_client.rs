//! BLE client for IDUN Guardian EEG earbuds.
//!
//! This module provides the primary API for interacting with Guardian earbuds:
//!
//! - [`GuardianClient`] — scan for and connect to Guardian devices
//! - [`GuardianHandle`] — send commands to a connected device (start/stop recording,
//!   impedance, LED control, battery reading)
//! - [`GuardianDevice`] — a discovered device from a BLE scan
//! - [`GuardianClientConfig`] — connection configuration (notch filter, scan timeout, name prefix)
//!
//! # Connection flow
//!
//! ```text
//! GuardianClient::new(config)
//!     ├── scan_all()       → Vec<GuardianDevice>
//!     │       └── connect_to(device) → (Receiver<GuardianEvent>, GuardianHandle)
//!     └── connect()        → (Receiver<GuardianEvent>, GuardianHandle)  [auto-picks first]
//! ```
//!
//! # Background tasks
//!
//! After connection, three `tokio` tasks run in the background:
//!
//! | Task | Purpose | Lifecycle |
//! |---|---|---|
//! | Notification dispatcher | Routes BLE notifications → `GuardianEvent` variants | Until disconnect |
//! | Battery poller | Reads battery every 60s | Until disconnect |
//! | Disconnect watcher | Detects BLE link loss | Until disconnect |
//!
//! # Example
//!
//! ```no_run
//! use idun::guardian_client::{GuardianClient, GuardianClientConfig};
//! use idun::types::GuardianEvent;
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! let client = GuardianClient::new(GuardianClientConfig::default());
//! let (mut rx, handle) = client.connect().await?;
//! handle.start_recording().await?;
//!
//! while let Some(event) = rx.recv().await {
//!     match event {
//!         GuardianEvent::Eeg(r) => println!("EEG #{}", r.index),
//!         GuardianEvent::Disconnected => break,
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use log::{debug, info, warn};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::protocol::*;
use crate::types::*;

// ── Timestamp helper ──────────────────────────────────────────────────────────

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_secs_f64()
        * 1000.0
}

// ── Per-packet timestamp tracker ─────────────────────────────────────────────

/// Reconstructs wall-clock timestamps from the Guardian's 8-bit packet index.
///
/// The Guardian earbud sends a monotonically increasing 8-bit index (0–255)
/// with each EEG packet. Each packet contains 20 samples at 250 Hz, so the
/// inter-packet interval is 20/250 = 80 ms.
struct TimestampTracker {
    last_index: Option<u8>,
    last_timestamp: Option<f64>,
}

impl TimestampTracker {
    fn new() -> Self {
        Self {
            last_index: None,
            last_timestamp: None,
        }
    }

    /// Return the wall-clock timestamp in ms since Unix epoch for the given packet index.
    fn get(&mut self, index: u8) -> f64 {
        let packet_duration_ms =
            1000.0 * (EEG_SAMPLES_PER_PACKET as f64) / EEG_SAMPLE_RATE;

        if self.last_index.is_none() || self.last_timestamp.is_none() {
            self.last_index = Some(index);
            self.last_timestamp = Some(now_ms());
            return self.last_timestamp.unwrap();
        }

        let prev = self.last_index.unwrap() as i32;
        let curr = index as i32;

        // Handle 8-bit wrap-around (0–255)
        let delta = ((curr - prev) + MAX_PACKET_INDEX as i32) % MAX_PACKET_INDEX as i32;

        let new_ts = self.last_timestamp.unwrap() + packet_duration_ms * delta as f64;
        self.last_index = Some(index);
        self.last_timestamp = Some(new_ts);
        new_ts
    }

    fn reset(&mut self) {
        self.last_index = None;
        self.last_timestamp = None;
    }
}

// ── GuardianDevice ────────────────────────────────────────────────────────────

/// A Guardian earbud discovered during a BLE scan.
#[derive(Clone, Debug)]
pub struct GuardianDevice {
    /// Advertised device name (e.g. `"IGEB"` or `"IGE-ABCDEF"`).
    pub name: String,
    /// Platform BLE identifier.
    pub id: String,
    pub(crate) peripheral: Peripheral,
    pub(crate) adapter: Adapter,
}

// ── GuardianClientConfig ──────────────────────────────────────────────────────

/// Configuration for [`GuardianClient`].
#[derive(Debug, Clone)]
pub struct GuardianClientConfig {
    /// Use 60 Hz mains frequency for the notch filter (Americas, Japan).
    /// Default: `false` (50 Hz — Europe, Asia).
    pub mains_freq_60hz: bool,
    /// BLE scan duration in seconds before giving up. Default: `15`.
    pub scan_timeout_secs: u64,
    /// Match devices whose advertised name starts with this string.
    /// Default: `"IGE"`.
    pub name_prefix: String,
}

impl Default for GuardianClientConfig {
    fn default() -> Self {
        Self {
            mains_freq_60hz: false,
            scan_timeout_secs: 15,
            name_prefix: DEVICE_NAME_PREFIX.into(),
        }
    }
}

// ── GuardianClient ────────────────────────────────────────────────────────────

/// BLE client for IDUN Guardian EEG earbuds.
///
/// Handles scanning, connecting, GATT subscription, and notification dispatch.
pub struct GuardianClient {
    config: GuardianClientConfig,
}

impl GuardianClient {
    pub fn new(config: GuardianClientConfig) -> Self {
        Self { config }
    }

    // ── Public: scan ─────────────────────────────────────────────────────────

    /// Scan for all nearby Guardian devices and return them.
    pub async fn scan_all(&self) -> Result<Vec<GuardianDevice>> {
        let manager = Manager::new().await?;
        let adapters = manager.adapters().await?;
        let adapter = adapters
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("No Bluetooth adapter found"))?;

        #[cfg(target_os = "macos")]
        {
            use btleplug::api::CentralState;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            loop {
                match adapter.adapter_state().await {
                    Ok(CentralState::PoweredOn) => break,
                    Ok(_) if tokio::time::Instant::now() >= deadline => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        info!(
            "scan_all: scanning for {} s …",
            self.config.scan_timeout_secs
        );
        adapter.start_scan(ScanFilter::default()).await?;
        tokio::time::sleep(Duration::from_secs(self.config.scan_timeout_secs)).await;
        adapter.stop_scan().await.ok();

        let mut found = vec![];
        for p in adapter.peripherals().await? {
            if let Ok(Some(props)) = p.properties().await {
                if let Some(name) = props.local_name {
                    if name.starts_with(&self.config.name_prefix) {
                        let id = p.id().to_string();
                        info!("scan_all: found {name}  id={id}");
                        found.push(GuardianDevice {
                            name,
                            id,
                            peripheral: p,
                            adapter: adapter.clone(),
                        });
                    }
                }
            }
        }
        info!("scan_all: {} device(s) found", found.len());
        Ok(found)
    }

    // ── Public: connect_to ────────────────────────────────────────────────────

    /// Connect to a specific device returned by [`GuardianClient::scan_all`].
    pub async fn connect_to(
        &self,
        device: GuardianDevice,
    ) -> Result<(mpsc::Receiver<GuardianEvent>, GuardianHandle)> {
        self.setup_peripheral(device.peripheral, device.name, device.adapter)
            .await
    }

    // ── Public: connect (convenience) ────────────────────────────────────────

    /// Scan for the first Guardian device, connect, and return the event channel.
    pub async fn connect(&self) -> Result<(mpsc::Receiver<GuardianEvent>, GuardianHandle)> {
        let manager = Manager::new().await?;
        let adapters = manager.adapters().await?;
        let adapter = adapters
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("No Bluetooth adapter found"))?;

        #[cfg(target_os = "macos")]
        {
            use btleplug::api::CentralState;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            loop {
                match adapter.adapter_state().await {
                    Ok(CentralState::PoweredOn) => break,
                    Ok(_) if tokio::time::Instant::now() >= deadline => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        info!(
            "Scanning for Guardian devices (timeout: {} s) …",
            self.config.scan_timeout_secs
        );
        adapter.start_scan(ScanFilter::default()).await?;
        let peripheral = self
            .find_first(&adapter, &self.config.name_prefix, self.config.scan_timeout_secs)
            .await?;
        adapter.stop_scan().await.ok();

        let props = peripheral.properties().await?.unwrap_or_default();
        let device_name = props.local_name.unwrap_or_else(|| "Unknown".into());
        info!("Found device: {device_name}");

        self.setup_peripheral(peripheral, device_name, adapter)
            .await
    }

    // ── Private: setup_peripheral ─────────────────────────────────────────────

    async fn setup_peripheral(
        &self,
        peripheral: Peripheral,
        device_name: String,
        adapter: Adapter,
    ) -> Result<(mpsc::Receiver<GuardianEvent>, GuardianHandle)> {
        tokio::time::timeout(Duration::from_secs(10), peripheral.connect())
            .await
            .map_err(|_| anyhow!("BLE connect() timed out after 10 s"))??;

        #[cfg(target_os = "linux")]
        tokio::time::sleep(Duration::from_millis(600)).await;

        tokio::time::timeout(Duration::from_secs(15), peripheral.discover_services())
            .await
            .map_err(|_| anyhow!("discover_services() timed out after 15 s"))??;
        info!("Connected and services discovered: {device_name}");

        let chars: BTreeSet<Characteristic> = peripheral.characteristics();

        let find_char = |uuid: Uuid| -> Result<Characteristic> {
            chars
                .iter()
                .find(|c| c.uuid == uuid)
                .cloned()
                .ok_or_else(|| anyhow!("Characteristic {uuid} not found"))
        };

        // ── Read device info ──────────────────────────────────────────────────
        let mac_address = match peripheral.read(&find_char(MAC_ID_CHARACTERISTIC)?).await {
            Ok(bytes) => {
                let mac = String::from_utf8_lossy(&bytes).to_string();
                info!("Device MAC: {mac}");
                mac.replace(':', "-")
            }
            Err(e) => {
                warn!("Could not read MAC address: {e}");
                String::new()
            }
        };

        let firmware_version =
            match peripheral.read(&find_char(FIRMWARE_VERSION_CHARACTERISTIC)?).await {
                Ok(bytes) => {
                    let fw = String::from_utf8_lossy(&bytes).to_string();
                    info!("Firmware: {fw}");
                    fw
                }
                Err(e) => {
                    warn!("Could not read firmware version: {e}");
                    String::new()
                }
            };

        let hardware_version =
            match peripheral.read(&find_char(HARDWARE_VERSION_CHARACTERISTIC)?).await {
                Ok(bytes) => {
                    let hw = String::from_utf8_lossy(&bytes).to_string();
                    info!("Hardware: {hw}");
                    hw
                }
                Err(e) => {
                    warn!("Could not read hardware version: {e}");
                    String::new()
                }
            };

        // ── Subscribe to EEG/IMU notifications ───────────────────────────────
        let eeg_char = find_char(EEG_IMU_CHARACTERISTIC)?;
        peripheral.subscribe(&eeg_char).await?;

        // ── Subscribe to impedance notifications ─────────────────────────────
        let impedance_char = find_char(IMPEDANCE_CHARACTERISTIC)?;
        // Don't subscribe yet — impedance streaming is started explicitly.

        // ── Find config and command characteristics ──────────────────────────
        let config_char = find_char(CONFIG_CHARACTERISTIC)?;
        let command_char = find_char(COMMAND_CHARACTERISTIC)?;

        // ── Event channel ─────────────────────────────────────────────────────
        let (tx, rx) = mpsc::channel::<GuardianEvent>(256);
        let _ = tx.send(GuardianEvent::Connected(device_name.clone())).await;
        let _ = tx
            .send(GuardianEvent::DeviceInfo(DeviceInfo {
                mac_address: mac_address.clone(),
                firmware_version,
                hardware_version,
            }))
            .await;

        // ── Read battery ──────────────────────────────────────────────────────
        if let Ok(batt_char) = find_char(BATTERY_CHARACTERISTIC) {
            match peripheral.read(&batt_char).await {
                Ok(bytes) if !bytes.is_empty() => {
                    let level = bytes[0];
                    info!("Battery: {level}%");
                    let _ = tx.send(GuardianEvent::Battery(BatteryReading { level })).await;
                }
                _ => {
                    warn!("Could not read battery level");
                }
            }
        }

        // ── Disconnect watcher ──────────────────────────────────────────────
        let disconnect_tx = tx.clone();
        let peripheral_id = peripheral.id();
        tokio::spawn(async move {
            match adapter.events().await {
                Ok(mut events) => {
                    while let Some(event) = events.next().await {
                        if let CentralEvent::DeviceDisconnected(id) = event {
                            if id == peripheral_id {
                                info!("Disconnect watcher: device disconnected.");
                                let _ = disconnect_tx.send(GuardianEvent::Disconnected).await;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Disconnect watcher: could not subscribe to adapter events: {e}");
                }
            }
        });

        // ── Battery polling task (every 60 s) ─────────────────────────────
        if let Ok(batt_char) = find_char(BATTERY_CHARACTERISTIC) {
            let batt_peripheral = peripheral.clone();
            let batt_tx = tx.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(60));
                interval.tick().await; // skip first (we already read at connect)
                loop {
                    interval.tick().await;
                    match batt_peripheral.read(&batt_char).await {
                        Ok(bytes) if !bytes.is_empty() => {
                            let level = bytes[0];
                            debug!("Battery poll: {level}%");
                            if batt_tx
                                .send(GuardianEvent::Battery(BatteryReading { level }))
                                .await
                                .is_err()
                            {
                                break; // channel closed
                            }
                        }
                        Ok(_) => {}
                        Err(_) => break, // device likely disconnected
                    }
                }
            });
        }

        // ── Notification dispatch task ───────────────────────────────────────
        let peripheral_clone = peripheral.clone();
        tokio::spawn(async move {
            let mut notifications = match peripheral_clone.notifications().await {
                Ok(n) => n,
                Err(e) => {
                    warn!("Could not get notifications stream: {e}");
                    return;
                }
            };
            info!("Notification stream subscribed, waiting for data…");

            let mut ts_tracker = TimestampTracker::new();
            let mut notif_count: u64 = 0;

            while let Some(notif) = notifications.next().await {
                let data = &notif.value;
                let uuid = notif.uuid;
                notif_count += 1;

                if notif_count <= 5 || notif_count % 500 == 0 {
                    debug!(
                        "Notification #{notif_count} uuid={uuid} len={}",
                        data.len()
                    );
                }

                if uuid == EEG_IMU_CHARACTERISTIC {
                    if data.len() >= 2 {
                        let index = data[1];
                        let timestamp = ts_tracker.get(index);

                        // Always emit the raw EEG packet
                        let _ = tx
                            .send(GuardianEvent::Eeg(EegReading {
                                index,
                                timestamp,
                                raw_data: data.to_vec(),
                                samples: None,
                                decode_source: crate::types::DecodeSource::None,
                            }))
                            .await;

                        // Try to extract IMU data from the packet
                        // The EEG+IMU characteristic multiplexes both;
                        // IMU may be appended after EEG samples.
                        #[cfg(feature = "local-decode")]
                        if data.len() >= 14 {
                            // Try last 12 bytes as accel[3]+gyro[3] i16 LE
                            if let Some((accel, gyro)) =
                                crate::parse::try_decode_imu_i16le(&data[data.len() - 12..])
                            {
                                let _ = tx
                                    .send(GuardianEvent::Accelerometer(
                                        crate::types::AccelerometerReading {
                                            index,
                                            timestamp,
                                            sample: accel,
                                        },
                                    ))
                                    .await;
                                let _ = tx
                                    .send(GuardianEvent::Gyroscope(
                                        crate::types::GyroscopeReading {
                                            index,
                                            timestamp,
                                            sample: gyro,
                                        },
                                    ))
                                    .await;
                            }
                        }
                    }
                    continue;
                }

                if uuid == IMPEDANCE_CHARACTERISTIC {
                    let impedance_ohms = if data.len() >= 4 {
                        u32::from_le_bytes([data[0], data[1], data[2], data[3]])
                    } else if data.len() >= 2 {
                        u16::from_le_bytes([data[0], data[1]]) as u32
                    } else if !data.is_empty() {
                        data[0] as u32
                    } else {
                        continue;
                    };
                    let impedance_kohms = impedance_ohms as f64 / 1000.0;
                    let _ = tx
                        .send(GuardianEvent::Impedance(ImpedanceReading {
                            impedance_ohms,
                            impedance_kohms,
                            timestamp: now_ms(),
                        }))
                        .await;
                    continue;
                }

                debug!("Unknown notification from {uuid}");
            }

            info!("Notification stream ended – device disconnected.");
            let _ = tx.send(GuardianEvent::Disconnected).await;
            ts_tracker.reset();
        });

        let handle = GuardianHandle {
            peripheral,
            config_char,
            command_char,
            impedance_char,
            mac_address,
            mains_freq_60hz: self.config.mains_freq_60hz,
        };

        Ok((rx, handle))
    }

    // ── Private: find_first ───────────────────────────────────────────────────

    async fn find_first(
        &self,
        adapter: &Adapter,
        prefix: &str,
        timeout_secs: u64,
    ) -> Result<Peripheral> {
        use tokio::time::{sleep, timeout};

        let result = timeout(Duration::from_secs(timeout_secs), async {
            loop {
                let peripherals = adapter.peripherals().await.unwrap_or_default();
                for p in peripherals {
                    if let Ok(Some(props)) = p.properties().await {
                        if let Some(name) = &props.local_name {
                            if name.starts_with(prefix) {
                                return p;
                            }
                        }
                    }
                }
                sleep(Duration::from_millis(250)).await;
            }
        })
        .await;

        result.map_err(|_| {
            anyhow!("Timed out scanning for a Guardian device after {timeout_secs} s")
        })
    }
}

// ── GuardianHandle ────────────────────────────────────────────────────────────

/// A handle to an active Guardian connection for sending commands.
pub struct GuardianHandle {
    peripheral: Peripheral,
    config_char: Characteristic,
    command_char: Characteristic,
    impedance_char: Characteristic,
    /// Device MAC address (format: "AA-BB-CC-DD-EE-FF").
    pub mac_address: String,
    mains_freq_60hz: bool,
}

impl GuardianHandle {
    /// Start EEG/IMU recording on the earbud.
    pub async fn start_recording(&self) -> Result<()> {
        info!("Sending start measurement command");
        self.peripheral
            .write(&self.command_char, CMD_START_MEASUREMENT, WriteType::WithoutResponse)
            .await?;
        Ok(())
    }

    /// Stop EEG/IMU recording on the earbud.
    pub async fn stop_recording(&self) -> Result<()> {
        info!("Sending stop measurement command");
        self.peripheral
            .write(&self.command_char, CMD_STOP_MEASUREMENT, WriteType::WithoutResponse)
            .await?;
        Ok(())
    }

    /// Start impedance streaming.
    ///
    /// First subscribes to the impedance characteristic notifications,
    /// configures the notch filter, then sends the start impedance command.
    pub async fn start_impedance(&self) -> Result<()> {
        info!("Starting impedance streaming");
        self.peripheral.subscribe(&self.impedance_char).await?;

        let notch_cfg = if self.mains_freq_60hz {
            CFG_NOTCH_60HZ
        } else {
            CFG_NOTCH_50HZ
        };
        self.peripheral
            .write(&self.config_char, notch_cfg, WriteType::WithoutResponse)
            .await?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        self.peripheral
            .write(&self.command_char, CMD_START_IMPEDANCE, WriteType::WithoutResponse)
            .await?;
        Ok(())
    }

    /// Stop impedance streaming.
    pub async fn stop_impedance(&self) -> Result<()> {
        info!("Stopping impedance streaming");
        self.peripheral
            .write(&self.command_char, CMD_STOP_IMPEDANCE, WriteType::WithoutResponse)
            .await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        self.peripheral.unsubscribe(&self.impedance_char).await?;
        Ok(())
    }

    /// Turn the LED off (useful during long recordings).
    pub async fn led_off(&self) -> Result<()> {
        self.peripheral
            .write(&self.config_char, CFG_LED_OFF, WriteType::WithoutResponse)
            .await?;
        Ok(())
    }

    /// Turn the LED on.
    pub async fn led_on(&self) -> Result<()> {
        self.peripheral
            .write(&self.config_char, CFG_LED_ON, WriteType::WithoutResponse)
            .await?;
        Ok(())
    }

    /// Read the current battery level.
    pub async fn read_battery(&self) -> Result<u8> {
        let chars: BTreeSet<Characteristic> = self.peripheral.characteristics();
        let batt_char = chars
            .iter()
            .find(|c| c.uuid == BATTERY_CHARACTERISTIC)
            .ok_or_else(|| anyhow!("Battery characteristic not found"))?;
        let data = self.peripheral.read(batt_char).await?;
        Ok(if data.is_empty() { 0 } else { data[0] })
    }

    /// Check if the peripheral is still connected.
    pub async fn is_connected(&self) -> bool {
        self.peripheral.is_connected().await.unwrap_or(false)
    }

    /// Gracefully disconnect.
    pub async fn disconnect(&self) -> Result<()> {
        self.peripheral.disconnect().await?;
        Ok(())
    }
}
