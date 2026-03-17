//! IDUN Cloud WebSocket client for server-side EEG decoding.
//!
//! When the local experimental 12-bit decoder fails (or is not trusted),
//! this module provides a fallback that sends raw BLE packets to the
//! IDUN Cloud API for authoritative decoding.
//!
//! # API token
//!
//! You need an **IDUN API token** to use the cloud decoder.
//! Get one from <https://idun.tech/>, then provide it using one of:
//!
//! ### Option 1: Environment variable (recommended)
//!
//! ```bash
//! export IDUN_API_TOKEN="your_api_token"
//! ```
//!
//! Then use [`CloudDecoder::from_env`]:
//!
//! ```rust,no_run
//! # use idun::cloud::CloudDecoder;
//! let decoder = CloudDecoder::from_env("AA-BB-CC-DD-EE-FF".into())?;
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! ### Option 2: Pass directly
//!
//! ```rust
//! # use idun::cloud::CloudDecoder;
//! let decoder = CloudDecoder::new(
//!     "your_api_token".into(),
//!     "AA-BB-CC-DD-EE-FF".into(),
//! );
//! ```
//!
//! ### Option 3: CLI flag
//!
//! ```bash
//! cargo run --bin idun -- --cloud --token your_api_token
//! ```
//!
//! > **Security**: Keep your API token private. Do not commit it to version
//! > control. Use environment variables or a secrets manager in production.
//!
//! # Protocol
//!
//! The IDUN Cloud uses a WebSocket endpoint at `wss://ws-api.idun.cloud`.
//! Authentication is via query parameter: `?authorization={api_token}`.
//!
//! ## Session flow
//!
//! ```text
//! Client                              IDUN Cloud
//! ──────                              ──────────
//!   ──── WebSocket + auth token ────────▶
//!   ──── startNewRecording ─────────────▶
//!   ◀──── recordingUpdate (recordingId) ──
//!   ──── subscribeLiveStreamInsights ───▶
//!   ──── publishRawMeasurements ────────▶ (base64-encoded BLE packets)
//!   ◀──── liveStreamInsights ──────────── (decoded EEG samples)
//!   ...repeat...
//!   ──── endOngoingRecording ───────────▶
//! ```
//!
//! ## Available stream types
//!
//! | Type | Description |
//! |---|---|
//! | `RAW_EEG` | Unfiltered EEG samples |
//! | `FILTERED_EEG` | Bandpass-filtered EEG |
//! | `IMU` | Accelerometer + gyroscope |
//!
//! # Usage
//!
//! ```no_run
//! use idun::cloud::CloudDecoder;
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! // Create decoder (from env or explicit token)
//! let mut decoder = CloudDecoder::new(
//!     "my-api-token".to_string(),
//!     "AA-BB-CC-DD-EE-FF".to_string(),
//! );
//!
//! // Connect and start a recording session
//! decoder.connect().await?;
//!
//! // Send a raw BLE packet for cloud decoding
//! let raw_packet = vec![0xAA, 0x01, /* ... */];
//! decoder.send_raw_packet(&raw_packet, 1234567890.0, 0).await?;
//!
//! // Receive decoded data (blocking)
//! if let Some(decoded) = decoder.recv_decoded().await? {
//!     println!("Decoded: {:?}", decoded);
//! }
//!
//! // Or non-blocking poll
//! if let Some(decoded) = decoder.try_recv_decoded()? {
//!     println!("Polled: {:?}", decoded);
//! }
//!
//! // Clean shutdown
//! decoder.disconnect().await?;
//! # Ok(())
//! # }
//! ```

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const WS_ENDPOINT: &str = "wss://ws-api.idun.cloud";
const PLATFORM: &str = "SDK_RUST";

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Decoded EEG data received from the IDUN Cloud.
#[derive(Debug, Clone)]
pub struct CloudDecodedEeg {
    /// The action that produced this data (e.g. `"liveStreamInsights"`).
    pub action: String,
    /// The full JSON message from the cloud (contains decoded samples,
    /// stream type, timestamps, etc.).
    pub data: Value,
    /// Sequence number of the original packet that was sent.
    pub sequence: Option<u64>,
}

/// State of the cloud recording session.
#[derive(Debug, Clone, PartialEq)]
enum SessionState {
    /// Not connected.
    Disconnected,
    /// WebSocket connected, waiting for recording to start.
    Connected,
    /// Recording started, waiting for data flow.
    RecordingStarted {
        recording_id: String,
    },
    /// Recording is ongoing, data is flowing.
    RecordingOngoing {
        recording_id: String,
    },
    /// Session ended.
    Ended,
}

/// IDUN Cloud WebSocket client for server-side EEG packet decoding.
///
/// Manages the full lifecycle: connect → start recording → subscribe to
/// live insights → send raw packets → receive decoded data → end recording.
pub struct CloudDecoder {
    api_token: String,
    device_id: String,
    state: SessionState,
    /// Channel to send raw messages to the WebSocket writer task.
    ws_tx: Option<mpsc::Sender<String>>,
    /// Channel to receive decoded events from the WebSocket reader task.
    decoded_rx: Option<mpsc::Receiver<CloudDecodedEeg>>,
    /// Sequence counter for outgoing packets.
    sequence: u64,
    /// Whether the live insights subscription has been sent.
    subscribed: bool,
}

impl CloudDecoder {
    /// Create a new cloud decoder.
    ///
    /// # Arguments
    /// * `api_token` — IDUN API token (or set `IDUN_API_TOKEN` env var)
    /// * `device_id` — Guardian MAC address (format: `"AA-BB-CC-DD-EE-FF"`)
    pub fn new(api_token: String, device_id: String) -> Self {
        Self {
            api_token,
            device_id,
            state: SessionState::Disconnected,
            ws_tx: None,
            decoded_rx: None,
            sequence: 0,
            subscribed: false,
        }
    }

    /// Create a cloud decoder from the `IDUN_API_TOKEN` environment variable.
    ///
    /// Returns `Err` if the env var is not set.
    pub fn from_env(device_id: String) -> Result<Self> {
        let token = std::env::var("IDUN_API_TOKEN")
            .map_err(|_| anyhow!("IDUN_API_TOKEN environment variable not set. \
                Set it with: export IDUN_API_TOKEN=your-token"))?;
        Ok(Self::new(token, device_id))
    }

    /// Connect to the IDUN Cloud WebSocket and start a recording session.
    ///
    /// This method:
    /// 1. Opens the WebSocket connection with authentication
    /// 2. Spawns reader/writer tasks
    /// 3. Sends `startNewRecording`
    /// 4. Waits for the recording ID to be assigned
    /// 5. Subscribes to `FILTERED_EEG` live stream insights
    pub async fn connect(&mut self) -> Result<()> {
        let ws_url = format!("{}?authorization={}", WS_ENDPOINT, self.api_token);

        info!("[CLOUD] Connecting to IDUN Cloud at {WS_ENDPOINT}…");
        let (ws_stream, _response) = connect_async(&ws_url).await
            .map_err(|e| anyhow!("Failed to connect to IDUN Cloud: {e}"))?;
        info!("[CLOUD] WebSocket connected");

        let (mut ws_write, mut ws_read) = ws_stream.split();

        // Channel for outgoing messages (to the WebSocket writer)
        let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
        // Channel for decoded events (from the WebSocket reader)
        let (decoded_tx, decoded_rx) = mpsc::channel::<CloudDecodedEeg>(256);

        self.ws_tx = Some(out_tx.clone());
        self.decoded_rx = Some(decoded_rx);
        self.state = SessionState::Connected;

        // ── Writer task: drain out_tx and send to WebSocket ──────────────
        tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                if let Err(e) = ws_write.send(Message::Text(msg.into())).await {
                    error!("[CLOUD] WebSocket write error: {e}");
                    break;
                }
            }
            debug!("[CLOUD] Writer task ended");
        });

        // ── Reader task: read from WebSocket and dispatch ────────────────
        let state_tx = mpsc::channel::<(String, String)>(16);
        let mut state_rx = state_tx.1;
        let recording_state_tx = state_tx.0;

        tokio::spawn(async move {
            while let Some(msg) = ws_read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        // Cloud sends base64-encoded JSON
                        let json_str = if let Ok(decoded_bytes) =
                            base64::engine::general_purpose::STANDARD.decode(text.as_bytes())
                        {
                            String::from_utf8_lossy(&decoded_bytes).to_string()
                        } else {
                            // Maybe it's plain JSON
                            text.to_string()
                        };

                        match serde_json::from_str::<Value>(&json_str) {
                            Ok(event) => {
                                let action = event
                                    .get("action")
                                    .and_then(|a| a.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                match action.as_str() {
                                    "recordingUpdate" => {
                                        let status = event
                                            .get("message")
                                            .and_then(|m| m.get("status"))
                                            .and_then(|s| s.as_str())
                                            .unwrap_or("");
                                        let rec_id = event
                                            .get("message")
                                            .and_then(|m| m.get("recordingId"))
                                            .and_then(|r| r.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        info!("[CLOUD] Recording update: status={status} id={rec_id}");
                                        let _ = recording_state_tx
                                            .send((status.to_string(), rec_id))
                                            .await;
                                    }
                                    "liveStreamInsights" => {
                                        let seq = event.get("sequence")
                                            .and_then(|s| s.as_u64());
                                        let _ = decoded_tx
                                            .send(CloudDecodedEeg {
                                                action: action.clone(),
                                                data: event,
                                                sequence: seq,
                                            })
                                            .await;
                                    }
                                    "realtimePredictionsResponse" => {
                                        let seq = event.get("sequence")
                                            .and_then(|s| s.as_u64());
                                        let _ = decoded_tx
                                            .send(CloudDecodedEeg {
                                                action: action.clone(),
                                                data: event,
                                                sequence: seq,
                                            })
                                            .await;
                                    }
                                    "clientError" => {
                                        let msg = event
                                            .get("message")
                                            .and_then(|m| m.as_str())
                                            .unwrap_or("unknown error");
                                        error!("[CLOUD] Client error: {msg}");
                                    }
                                    other => {
                                        debug!("[CLOUD] Unhandled action: {other}");
                                    }
                                }
                            }
                            Err(e) => {
                                debug!("[CLOUD] JSON parse error: {e} | raw: {json_str}");
                            }
                        }
                    }
                    Ok(Message::Binary(bin)) => {
                        // Try base64 decode then JSON parse
                        if let Ok(decoded) =
                            base64::engine::general_purpose::STANDARD.decode(&bin)
                        {
                            let json_str = String::from_utf8_lossy(&decoded);
                            debug!("[CLOUD] Binary message: {json_str}");
                        }
                    }
                    Ok(Message::Close(_)) => {
                        info!("[CLOUD] WebSocket closed by server");
                        break;
                    }
                    Err(e) => {
                        error!("[CLOUD] WebSocket read error: {e}");
                        break;
                    }
                    _ => {}
                }
            }
            debug!("[CLOUD] Reader task ended");
        });

        // ── Send startNewRecording ───────────────────────────────────────
        let start_msg = json!({
            "version": 1,
            "platform": PLATFORM,
            "action": "startNewRecording",
            "deviceId": self.device_id,
            "deviceTs": now_ms(),
        });
        self.send_raw_json(&start_msg).await?;
        info!("[CLOUD] Sent startNewRecording");

        // ── Wait for recording ID ────────────────────────────────────────
        let timeout = tokio::time::Duration::from_secs(15);
        let mut recording_id = String::new();

        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            tokio::select! {
                Some((status, rec_id)) = state_rx.recv() => {
                    match status.as_str() {
                        "NOT_STARTED" => {
                            recording_id = rec_id;
                            info!("[CLOUD] Recording ID assigned: {recording_id}");
                            self.state = SessionState::RecordingStarted {
                                recording_id: recording_id.clone(),
                            };
                        }
                        "ONGOING" => {
                            if recording_id.is_empty() {
                                recording_id = rec_id;
                            }
                            info!("[CLOUD] Recording is ONGOING");
                            self.state = SessionState::RecordingOngoing {
                                recording_id: recording_id.clone(),
                            };
                            break;
                        }
                        "COMPLETED" | "FAILED" => {
                            warn!("[CLOUD] Recording ended with status: {status}");
                            self.state = SessionState::Ended;
                            return Err(anyhow!("Recording ended unexpectedly: {status}"));
                        }
                        _ => {
                            debug!("[CLOUD] Unexpected recording status: {status}");
                        }
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    // If we got a recording ID but never saw ONGOING, proceed anyway
                    if !recording_id.is_empty() {
                        info!("[CLOUD] Proceeding with recording ID (no ONGOING received)");
                        self.state = SessionState::RecordingOngoing {
                            recording_id: recording_id.clone(),
                        };
                        break;
                    }
                    return Err(anyhow!("Timed out waiting for recording ID from cloud"));
                }
            }
        }

        // ── Subscribe to live stream insights ────────────────────────────
        self.subscribe_live_insights(&recording_id).await?;

        // Spawn a task to keep draining state_rx so it doesn't block
        tokio::spawn(async move {
            while state_rx.recv().await.is_some() {}
        });

        Ok(())
    }

    /// Subscribe to FILTERED_EEG and RAW_EEG live stream insights.
    async fn subscribe_live_insights(&mut self, recording_id: &str) -> Result<()> {
        let msg = json!({
            "version": 1,
            "platform": PLATFORM,
            "action": "subscribeLiveStreamInsights",
            "deviceId": self.device_id,
            "deviceTs": now_ms(),
            "recordingId": recording_id,
            "streamsTypes": ["RAW_EEG", "FILTERED_EEG"],
        });
        self.send_raw_json(&msg).await?;
        self.subscribed = true;
        info!("[CLOUD] Subscribed to live stream insights (RAW_EEG, FILTERED_EEG)");
        Ok(())
    }

    /// Send a raw BLE packet to the cloud for decoding.
    ///
    /// The packet is base64-encoded and wrapped in a `publishRawMeasurements`
    /// message, matching the format used by the official Python SDK.
    ///
    /// Returns `Err` if the WebSocket is not connected or the send fails.
    pub async fn send_raw_packet(
        &mut self,
        raw_data: &[u8],
        device_ts: f64,
        sequence: u64,
    ) -> Result<()> {
        let recording_id = match &self.state {
            SessionState::RecordingOngoing { recording_id } => recording_id.clone(),
            SessionState::RecordingStarted { recording_id } => recording_id.clone(),
            _ => return Err(anyhow!("Cloud session not active (state: {:?})", self.state)),
        };

        let b64 = base64::engine::general_purpose::STANDARD.encode(raw_data);

        let msg = json!({
            "version": 1,
            "platform": PLATFORM,
            "action": "publishRawMeasurements",
            "deviceId": self.device_id,
            "deviceTs": device_ts as u64,
            "event": b64,
            "recordingId": recording_id,
            "sequence": sequence,
        });

        self.send_raw_json(&msg).await?;
        self.sequence = sequence + 1;
        Ok(())
    }

    /// Try to receive the next decoded EEG event from the cloud.
    ///
    /// Returns `Ok(None)` if no data is available yet (non-blocking).
    /// Returns `Ok(Some(decoded))` with the cloud-decoded data.
    /// Returns `Err` if the channel is closed (cloud disconnected).
    pub fn try_recv_decoded(&mut self) -> Result<Option<CloudDecodedEeg>> {
        if let Some(ref mut rx) = self.decoded_rx {
            match rx.try_recv() {
                Ok(decoded) => Ok(Some(decoded)),
                Err(mpsc::error::TryRecvError::Empty) => Ok(None),
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    Err(anyhow!("Cloud decoder channel closed"))
                }
            }
        } else {
            Ok(None)
        }
    }

    /// Receive the next decoded EEG event from the cloud (blocking).
    ///
    /// Returns `Ok(None)` if the cloud disconnected.
    pub async fn recv_decoded(&mut self) -> Result<Option<CloudDecodedEeg>> {
        if let Some(ref mut rx) = self.decoded_rx {
            Ok(rx.recv().await)
        } else {
            Ok(None)
        }
    }

    /// Send a raw JSON message to the cloud WebSocket.
    async fn send_raw_json(&self, msg: &Value) -> Result<()> {
        if let Some(ref tx) = self.ws_tx {
            let text = serde_json::to_string(msg)?;
            tx.send(text)
                .await
                .map_err(|e| anyhow!("Failed to send to cloud: {e}"))?;
            Ok(())
        } else {
            Err(anyhow!("WebSocket not connected"))
        }
    }

    /// End the recording session and close the WebSocket.
    pub async fn disconnect(&mut self) -> Result<()> {
        let recording_id = match &self.state {
            SessionState::RecordingOngoing { recording_id }
            | SessionState::RecordingStarted { recording_id } => recording_id.clone(),
            _ => {
                self.state = SessionState::Disconnected;
                self.ws_tx = None;
                self.decoded_rx = None;
                return Ok(());
            }
        };

        let msg = json!({
            "version": 1,
            "platform": PLATFORM,
            "action": "endOngoingRecording",
            "deviceId": self.device_id,
            "deviceTs": now_ms(),
            "recordingId": recording_id,
        });

        if let Err(e) = self.send_raw_json(&msg).await {
            warn!("[CLOUD] Error sending endOngoingRecording: {e}");
        } else {
            info!("[CLOUD] Sent endOngoingRecording for {recording_id}");
        }

        // Give the server a moment to process
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        self.state = SessionState::Ended;
        self.ws_tx = None;
        self.decoded_rx = None;
        self.subscribed = false;

        info!("[CLOUD] Disconnected from IDUN Cloud");
        Ok(())
    }

    /// Check if the cloud session is active and ready to send/receive data.
    pub fn is_connected(&self) -> bool {
        matches!(
            self.state,
            SessionState::RecordingOngoing { .. } | SessionState::RecordingStarted { .. }
        )
    }

    /// Get the current recording ID, if a session is active.
    pub fn recording_id(&self) -> Option<&str> {
        match &self.state {
            SessionState::RecordingOngoing { recording_id }
            | SessionState::RecordingStarted { recording_id } => Some(recording_id),
            _ => None,
        }
    }
}
