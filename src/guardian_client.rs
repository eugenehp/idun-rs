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
//! A fourth forwards impedance notifications between
//! [`GuardianHandle::start_impedance`] and [`GuardianHandle::stop_impedance`].
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

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use log::{debug, info, warn};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use uuid::Uuid;
use webbluetooth::chooser::{Candidate, FirstMatch};
use webbluetooth::stream::{BoxStream, StreamExt};
use webbluetooth::uuid::BluetoothUuid;
use webbluetooth::{
    Bluetooth, BluetoothDevice, DeviceFilter, Grant, LeScanOptions, RemoteGattCharacteristic,
    RemoteGattServer, RequestDeviceOptions,
};

use crate::protocol::*;
use crate::types::*;

// ── GATT helpers ──────────────────────────────────────────────────────────────

// The adapter session is `Bluetooth::shared()`: one per process, cloned by
// every caller. It has to be shared, because a `GuardianDevice` is only usable
// through the session that found it and the TUI scans with one
// `GuardianClient` and connects with another.
//
// The `Uuid` constants in `crate::protocol` are passed to webbluetooth as they
// are, without conversion, through its `uuid` feature.

/// Every service the client reaches: the vendor service, Device Information
/// and Battery.
const GUARDIAN_SERVICES: [Uuid; 3] = [GUARDIAN_SERVICE, DEVICE_INFO_SERVICE, BATTERY_SERVICE];

/// What a device request asks for: Guardian earbuds by name, plus access to
/// the services their characteristics live under.
///
/// The services are named even though the filter matches on the name, because
/// a Web Bluetooth grant covers exactly the services a request lists and
/// `get_primary_service` refuses everything else.
fn request_options(name_prefix: &str) -> Result<RequestDeviceOptions> {
    Ok(RequestDeviceOptions::new()
        .filter(DeviceFilter::new().name_prefix(name_prefix))
        .optional_services(GUARDIAN_SERVICES)?)
}

/// What a scan reports: advertisements from anything named like a Guardian.
///
/// A scan grants nothing, so there is no service to name here — only what
/// should be reported.
fn scan_options(name_prefix: &str) -> LeScanOptions {
    LeScanOptions::new().filter(DeviceFilter::new().name_prefix(name_prefix))
}

/// What adopting a scanned device grants: the three Guardian services, and no
/// manufacturer data.
fn guardian_grant() -> Result<Grant> {
    Ok(Grant::new().services(GUARDIAN_SERVICES)?)
}

/// The MAC as the earbud reports it on [`DEVICE_ID_CHARACTERISTIC`]:
/// `"AA:BB:CC:DD:EE:FF"` plus a one-character terminator, which is dropped
/// before the colons become dashes.
fn parse_device_id(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes);
    let mut chars = raw.chars();
    chars.next_back();
    chars.as_str().replace(':', "-")
}

/// Pick one characteristic out of a device's discovered set.
fn find_char(chars: &[RemoteGattCharacteristic], uuid: Uuid) -> Result<RemoteGattCharacteristic> {
    let want = BluetoothUuid::from(uuid);
    chars
        .iter()
        .find(|c| *c.uuid() == want)
        .cloned()
        .ok_or_else(|| anyhow!("Characteristic {uuid} not found"))
}

/// Read a characteristic as a UTF-8 string, logging rather than failing when
/// it is missing or unreadable — device info is informational.
async fn read_string(chars: &[RemoteGattCharacteristic], uuid: Uuid, what: &str) -> String {
    let result = match find_char(chars, uuid) {
        Ok(c) => c.read_value().await.map_err(anyhow::Error::from),
        Err(e) => Err(e),
    };
    match result {
        Ok(bytes) => {
            let value = String::from_utf8_lossy(&bytes).to_string();
            info!("{what}: {value}");
            value
        }
        Err(e) => {
            warn!("Could not read {what}: {e}");
            String::new()
        }
    }
}

/// Subscribe to a characteristic and tag each notification with its UUID.
///
/// Web Bluetooth subscribes per characteristic and yields bare values, so the
/// UUID is attached here for the dispatch loop to route on.
async fn subscribe_tagged(
    characteristic: &RemoteGattCharacteristic,
    uuid: Uuid,
) -> Result<BoxStream<'static, (Uuid, Vec<u8>)>> {
    let notifications = characteristic.start_notifications().await?;
    Ok(notifications.map(move |value| (uuid, value)).boxed())
}

/// Decode one impedance notification into a reading — see
/// [`parse_impedance`](crate::parse::parse_impedance) for the wire format.
fn impedance_reading(data: &[u8]) -> Option<ImpedanceReading> {
    let impedance_ohms = crate::parse::parse_impedance(data)?;
    Some(ImpedanceReading {
        impedance_ohms,
        impedance_kohms: impedance_ohms as f64 / 1000.0,
        timestamp: now_ms(),
    })
}

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
        let packet_duration_ms = 1000.0 * (EEG_SAMPLES_PER_PACKET as f64) / EEG_SAMPLE_RATE;

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
///
/// Returned by [`GuardianClient::scan_all`]; pass to
/// [`GuardianClient::connect_to`] to establish a streaming connection.
#[derive(Clone, Debug)]
pub struct GuardianDevice {
    /// Advertised device name (e.g. `"IGEB"` or `"IGE-ABCDEF"`).
    pub name: String,
    /// Platform BLE identifier.
    /// • macOS / iOS — a per-host UUID the system assigns; the same earbud
    ///   has a different one on a different machine, because Apple never
    ///   exposes the hardware address
    /// • elsewhere — the Bluetooth address (`AA:BB:CC:DD:EE:FF`)
    pub id: String,
    pub(crate) device: BluetoothDevice,
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
    /// IDUN Cloud API token used to create a [`crate::cloud::CloudDecoder`]
    /// without reading from an environment variable.
    /// `None` (the default) falls back to the `IDUN_API_TOKEN` env var.
    pub api_token: Option<String>,
}

impl Default for GuardianClientConfig {
    fn default() -> Self {
        Self {
            mains_freq_60hz: false,
            scan_timeout_secs: 15,
            name_prefix: DEVICE_NAME_PREFIX.into(),
            api_token: None,
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
    ///
    /// The scan runs for `config.scan_timeout_secs` seconds so that multiple
    /// devices in range can all be discovered before the function returns.
    ///
    /// Every device returned has already been granted access to its services,
    /// so [`GuardianClient::connect_to`] does not have to scan again.
    pub async fn scan_all(&self) -> Result<Vec<GuardianDevice>> {
        let bluetooth = Bluetooth::shared();

        // Waits for the adapter's first state report — what the old macOS
        // "wait for poweredOn" polling loop was doing by hand.
        bluetooth
            .availability()
            .await
            .context("Bluetooth is not usable")?;

        info!(
            "scan_all: scanning for {} s …",
            self.config.scan_timeout_secs
        );
        let mut scan = bluetooth
            .request_le_scan(scan_options(&self.config.name_prefix))
            .await?;

        // A scan reports advertisements, and an earbud advertises repeatedly;
        // keyed by id, each one is kept once.
        let mut seen: BTreeMap<String, Candidate> = BTreeMap::new();
        let window = Duration::from_secs(self.config.scan_timeout_secs);
        let _ = tokio::time::timeout(window, async {
            while let Some(candidate) = scan.next().await {
                if !seen.contains_key(&candidate.id) {
                    info!("scan_all: found {}  id={}", candidate.label(), candidate.id);
                }
                seen.insert(candidate.id.clone(), candidate);
            }
        })
        .await;
        scan.stop(); // stop the radio before connecting to anything

        // A scan grants nothing, so each sighting is adopted into a device
        // with the grant it will need once connected.
        let grant = guardian_grant()?;
        let mut found = vec![];
        for (id, candidate) in seen {
            let name = candidate.label().to_owned();
            match bluetooth.adopt_candidate(&candidate, grant.clone()).await {
                Ok(device) => found.push(GuardianDevice { name, id, device }),
                Err(e) => warn!("scan_all: {name} ({id}) could not be adopted: {e}"),
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
        self.setup_device(device.device, device.name).await
    }

    // ── Public: connect (convenience) ────────────────────────────────────────

    /// Scan for the first Guardian device, connect, and return the event channel.
    pub async fn connect(&self) -> Result<(mpsc::Receiver<GuardianEvent>, GuardianHandle)> {
        let bluetooth = Bluetooth::shared();
        bluetooth
            .availability()
            .await
            .context("Bluetooth is not usable")?;

        info!(
            "Scanning for Guardian devices (timeout: {} s) …",
            self.config.scan_timeout_secs
        );
        // `FirstMatch` returns as soon as something matching advertises, so
        // the timeout is a deadline rather than a fixed scan window.
        let chooser = FirstMatch::with_timeout(Duration::from_secs(self.config.scan_timeout_secs));
        let device = bluetooth
            .request_device_with(request_options(&self.config.name_prefix)?, &chooser)
            .await
            .map_err(|e| {
                anyhow!(
                    "Timed out scanning for a Guardian device after {} s: {e}",
                    self.config.scan_timeout_secs
                )
            })?;

        let device_name = device.name().unwrap_or_else(|| device.id().to_owned());
        info!("Found device: {device_name}");

        self.setup_device(device, device_name).await
    }

    // ── Private: setup_device ─────────────────────────────────────────────────

    /// Connect a device, read its info, subscribe to EEG/IMU notifications,
    /// spawn the background tasks, and return the event channel.
    async fn setup_device(
        &self,
        device: BluetoothDevice,
        device_name: String,
    ) -> Result<(mpsc::Receiver<GuardianEvent>, GuardianHandle)> {
        let gatt = device.gatt();

        // `connect()` has no deadline of its own — it waits for the peripheral
        // to come into range for as long as that takes — so one is imposed
        // here.  Ten seconds is generous for a BLE connection that usually
        // takes under two.
        tokio::time::timeout(Duration::from_secs(10), gatt.connect())
            .await
            .map_err(|_| anyhow!("BLE connect() timed out after 10 s"))??;

        // The Guardian's characteristics span the vendor service, Device
        // Information and Battery, so discovery covers all three.
        let chars = tokio::time::timeout(Duration::from_secs(15), gatt.get_characteristics())
            .await
            .map_err(|_| anyhow!("service discovery timed out after 15 s"))??;
        info!("Connected and services discovered: {device_name}");

        // ── Read device info ──────────────────────────────────────────────────
        // The MAC comes from the vendor device-ID characteristic: the standard
        // Serial Number String (`0x2A25`) is on the Web Bluetooth GATT
        // blocklist and never readable.
        let mac_address = match find_char(&chars, DEVICE_ID_CHARACTERISTIC) {
            Ok(c) => c.read_value().await.map_err(anyhow::Error::from),
            Err(e) => Err(e),
        }
        .map(|bytes| parse_device_id(&bytes));
        let mac_address = match mac_address {
            Ok(mac) => {
                info!("Device MAC: {mac}");
                mac
            }
            Err(e) => {
                warn!("Could not read MAC address: {e}");
                String::new()
            }
        };
        let firmware_version =
            read_string(&chars, FIRMWARE_VERSION_CHARACTERISTIC, "firmware version").await;
        let hardware_version =
            read_string(&chars, HARDWARE_VERSION_CHARACTERISTIC, "hardware version").await;

        // ── Find characteristics ─────────────────────────────────────────────
        let eeg_char = find_char(&chars, EEG_IMU_CHARACTERISTIC)?;
        // Impedance is subscribed when impedance streaming is started.
        let impedance_char = find_char(&chars, IMPEDANCE_CHARACTERISTIC)?;
        let config_char = find_char(&chars, CONFIG_CHARACTERISTIC)?;
        let command_char = find_char(&chars, COMMAND_CHARACTERISTIC)?;
        let battery_char = find_char(&chars, BATTERY_CHARACTERISTIC).ok();

        // ── Subscribe to EEG/IMU notifications ───────────────────────────────
        let mut notifications = subscribe_tagged(&eeg_char, EEG_IMU_CHARACTERISTIC).await?;

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
        if let Some(batt_char) = &battery_char {
            match batt_char.read_value().await {
                Ok(bytes) if !bytes.is_empty() => {
                    let level = bytes[0];
                    info!("Battery: {level}%");
                    let _ = tx
                        .send(GuardianEvent::Battery(BatteryReading { level }))
                        .await;
                }
                _ => {
                    warn!("Could not read battery level");
                }
            }
        }

        // ── Disconnect watcher ──────────────────────────────────────────────
        // Fires when the BLE link drops (earbud powered off, out of range,
        // and so on) — often sooner than the notification stream closes.
        let disconnect_tx = tx.clone();
        let device_id = device.id().to_owned();
        let mut disconnects = device.watch_disconnect();
        tokio::spawn(async move {
            if disconnects.next().await.is_some() {
                info!("Disconnect watcher: device {device_id} disconnected.");
                let _ = disconnect_tx.send(GuardianEvent::Disconnected).await;
            }
        });

        // ── Battery polling task (every 60 s) ─────────────────────────────
        if let Some(batt_char) = battery_char.clone() {
            let batt_tx = tx.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(60));
                interval.tick().await; // skip first (we already read at connect)
                loop {
                    interval.tick().await;
                    match batt_char.read_value().await {
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
        let notif_tx = tx.clone();
        tokio::spawn(async move {
            info!("Notification stream subscribed, waiting for data…");
            let tx = notif_tx;

            let mut ts_tracker = TimestampTracker::new();
            let mut notif_count: u64 = 0;

            // Ends when the device disconnects (webbluetooth ≥ 0.0.4).
            while let Some((uuid, data)) = notifications.next().await {
                let data = &data[..];
                notif_count += 1;

                if notif_count <= 5 || notif_count.is_multiple_of(500) {
                    debug!("Notification #{notif_count} uuid={uuid} len={}", data.len());
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

                debug!("Unknown notification from {uuid}");
            }

            info!("Notification stream ended – device disconnected.");
            let _ = tx.send(GuardianEvent::Disconnected).await;
            ts_tracker.reset();
        });

        let handle = GuardianHandle {
            gatt,
            config_char,
            command_char,
            impedance_char,
            battery_char,
            impedance_task: Mutex::new(None),
            tx,
            mac_address,
            mains_freq_60hz: self.config.mains_freq_60hz,
        };

        Ok((rx, handle))
    }
}

// ── GuardianHandle ────────────────────────────────────────────────────────────

/// A handle to an active Guardian connection for sending commands.
pub struct GuardianHandle {
    gatt: RemoteGattServer,
    config_char: RemoteGattCharacteristic,
    command_char: RemoteGattCharacteristic,
    impedance_char: RemoteGattCharacteristic,
    battery_char: Option<RemoteGattCharacteristic>,
    /// Forwards impedance notifications while impedance streaming is on.
    /// Web Bluetooth subscribes per characteristic, so impedance gets its own
    /// stream, started and stopped with the measurement; aborting the task
    /// drops the stream, which is what ends the subscription.
    impedance_task: Mutex<Option<JoinHandle<()>>>,
    /// The connection's event channel, for the impedance forwarder.
    tx: mpsc::Sender<GuardianEvent>,
    /// Device MAC address (format: "AA-BB-CC-DD-EE-FF"). Empty if it could
    /// not be read.
    pub mac_address: String,
    mains_freq_60hz: bool,
}

impl GuardianHandle {
    /// Start EEG/IMU recording on the earbud.
    pub async fn start_recording(&self) -> Result<()> {
        info!("Sending start measurement command");
        self.command_char
            .write_value_without_response(CMD_START_MEASUREMENT)
            .await?;
        Ok(())
    }

    /// Stop EEG/IMU recording on the earbud.
    pub async fn stop_recording(&self) -> Result<()> {
        info!("Sending stop measurement command");
        self.command_char
            .write_value_without_response(CMD_STOP_MEASUREMENT)
            .await?;
        Ok(())
    }

    /// Start impedance streaming.
    ///
    /// First subscribes to the impedance characteristic notifications,
    /// configures the notch filter, then sends the start impedance command.
    pub async fn start_impedance(&self) -> Result<()> {
        info!("Starting impedance streaming");
        {
            let mut task = self.impedance_task.lock().await;
            if task.is_none() {
                let mut notifications = self.impedance_char.start_notifications().await?;
                let tx = self.tx.clone();
                *task = Some(tokio::spawn(async move {
                    while let Some(data) = notifications.next().await {
                        if let Some(reading) = impedance_reading(&data) {
                            if tx.send(GuardianEvent::Impedance(reading)).await.is_err() {
                                break; // channel closed
                            }
                        }
                    }
                }));
            }
        }

        let notch_cfg = if self.mains_freq_60hz {
            CFG_NOTCH_60HZ
        } else {
            CFG_NOTCH_50HZ
        };
        self.config_char
            .write_value_without_response(notch_cfg)
            .await?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        self.command_char
            .write_value_without_response(CMD_START_IMPEDANCE)
            .await?;
        Ok(())
    }

    /// Stop impedance streaming.
    pub async fn stop_impedance(&self) -> Result<()> {
        info!("Stopping impedance streaming");
        self.command_char
            .write_value_without_response(CMD_STOP_IMPEDANCE)
            .await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(task) = self.impedance_task.lock().await.take() {
            task.abort();
        }
        Ok(())
    }

    /// Turn the LED off (useful during long recordings).
    pub async fn led_off(&self) -> Result<()> {
        self.config_char
            .write_value_without_response(CFG_LED_OFF)
            .await?;
        Ok(())
    }

    /// Turn the LED on.
    pub async fn led_on(&self) -> Result<()> {
        self.config_char
            .write_value_without_response(CFG_LED_ON)
            .await?;
        Ok(())
    }

    /// Read the current battery level.
    pub async fn read_battery(&self) -> Result<u8> {
        let batt_char = self
            .battery_char
            .as_ref()
            .ok_or_else(|| anyhow!("Battery characteristic not found"))?;
        let data = batt_char.read_value().await?;
        Ok(if data.is_empty() { 0 } else { data[0] })
    }

    /// Check if the peripheral is still connected.
    pub async fn is_connected(&self) -> bool {
        self.gatt.connected()
    }

    /// Gracefully disconnect.
    pub async fn disconnect(&self) -> Result<()> {
        self.gatt.disconnect();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::parse_device_id;

    #[test]
    fn device_id_drops_terminator_and_dashes_colons() {
        assert_eq!(parse_device_id(b"AA:BB:CC:DD:EE:FF\0"), "AA-BB-CC-DD-EE-FF");
    }

    #[test]
    fn device_id_empty_is_empty() {
        assert_eq!(parse_device_id(b""), "");
    }
}
