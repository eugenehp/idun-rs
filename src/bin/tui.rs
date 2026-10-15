//! Real-time EEG / IMU / impedance viewer for IDUN Guardian earbuds.
//!
//! # Usage
//!
//! ```bash
//! # Real device — scan and auto-connect
//! cargo run --bin idun-tui
//!
//! # Simulated data (no hardware needed, requires simulate feature)
//! cargo run --bin idun-tui --features simulate -- --simulate
//!
//! # 60 Hz notch filter (Americas, Japan)
//! cargo run --bin idun-tui -- --60hz
//! ```
//!
//! # IDUN Cloud API token (optional, for cloud decoding)
//!
//! If you want cloud-decoded EEG data, set the `IDUN_API_TOKEN` environment
//! variable. Get a token from <https://idun.tech/>.
//!
//! ```bash
//! export IDUN_API_TOKEN="your_api_token"
//! ```
//!
//! Keys
//! ----
//!   Tab      open device picker
//!   1        EEG view (single channel)
//!   2        IMU view (accel + gyro, 6 axes)
//!   3        Impedance view
//!   4        All-in-one view (EEG + accel + gyro + impedance)
//!   s        trigger a fresh BLE scan
//!   +  / =   zoom out  (increase µV scale, EEG only)
//!   -        zoom in   (decrease µV scale, EEG only)
//!   a        auto-scale
//!   v        toggle smooth overlay
//!   p        pause / stop measurement
//!   r        resume / start measurement
//!   z        start impedance streaming
//!   c        clear waveform buffers
//!   d        disconnect current device
//!   q / Esc  quit

use std::collections::VecDeque;
use std::f64::consts::PI;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Chart, Clear, Dataset, GraphType, List, ListItem, ListState,
        Paragraph,
    },
    Frame, Terminal,
};
use tokio::sync::{mpsc, oneshot};

use idun::guardian_client::{
    GuardianClient, GuardianClientConfig, GuardianDevice, GuardianHandle,
};
#[cfg(feature = "local-decode")]
use idun::parse::try_decode_eeg_12bit;
use idun::protocol::{EEG_SAMPLE_RATE, EEG_SAMPLES_PER_PACKET};
use idun::types::GuardianEvent;

// ── Constants ─────────────────────────────────────────────────────────────────

const WINDOW_SECS: f64 = 4.0;
const EEG_HZ: f64 = EEG_SAMPLE_RATE;
const BUF_SIZE: usize = (WINDOW_SECS * EEG_HZ) as usize; // 1000
const IMU_HZ: f64 = 52.0; // typical MEMS IMU rate
const IMU_BUF_SIZE: usize = (WINDOW_SECS * IMU_HZ) as usize; // 208
const IMP_BUF_SIZE: usize = 120;

const Y_SCALES: &[f64] = &[10.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0];
const DEFAULT_SCALE: usize = 5;
const SMOOTH_WINDOW: usize = 9;
const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const RETRY_SECS: u64 = 6;
const RECONNECT_DELAY_SECS: u64 = 2;

// ── App mode ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub enum AppMode {
    Scanning,
    Connecting(String),
    Connected { name: String, id: String },
    Simulated,
    NoDevices,
    Disconnected,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Eeg,
    Imu,
    Impedance,
    All,
}

// ── App state ─────────────────────────────────────────────────────────────────

pub struct App {
    // EEG (1 channel)
    eeg_buf: VecDeque<f64>,

    // IMU — 3 axes each for accelerometer and gyroscope
    accel_x: VecDeque<f64>,
    accel_y: VecDeque<f64>,
    accel_z: VecDeque<f64>,
    gyro_x: VecDeque<f64>,
    gyro_y: VecDeque<f64>,
    gyro_z: VecDeque<f64>,

    // Latest IMU values for header display
    pub last_accel: Option<(f32, f32, f32)>,
    pub last_gyro: Option<(f32, f32, f32)>,

    // Impedance
    imp_buf: VecDeque<f64>,
    pub last_impedance_kohms: Option<f64>,

    // Status
    pub view: ViewMode,
    pub mode: AppMode,
    pub battery: Option<u8>,
    pub mac_address: Option<String>,
    pub firmware: Option<String>,
    pub hardware: Option<String>,

    // Rate tracking
    total_packets: u64,
    pkt_times: VecDeque<Instant>,

    // UI controls
    scale_idx: usize,
    pub paused: bool,
    pub smooth: bool,

    // Device picker
    pub show_picker: bool,
    pub picker_cursor: usize,
    pub picker_entries: Vec<String>,
    pub picker_connected_idx: Option<usize>,
    pub picker_scanning: bool,
    pub last_error: Option<String>,
}

impl App {
    fn new() -> Self {
        Self {
            eeg_buf: VecDeque::with_capacity(BUF_SIZE + 64),
            accel_x: VecDeque::with_capacity(IMU_BUF_SIZE + 16),
            accel_y: VecDeque::with_capacity(IMU_BUF_SIZE + 16),
            accel_z: VecDeque::with_capacity(IMU_BUF_SIZE + 16),
            gyro_x: VecDeque::with_capacity(IMU_BUF_SIZE + 16),
            gyro_y: VecDeque::with_capacity(IMU_BUF_SIZE + 16),
            gyro_z: VecDeque::with_capacity(IMU_BUF_SIZE + 16),
            last_accel: None,
            last_gyro: None,
            imp_buf: VecDeque::with_capacity(IMP_BUF_SIZE + 16),
            last_impedance_kohms: None,
            view: ViewMode::All,
            mode: AppMode::Scanning,
            battery: None,
            mac_address: None,
            firmware: None,
            hardware: None,
            total_packets: 0,
            pkt_times: VecDeque::with_capacity(256),
            scale_idx: DEFAULT_SCALE,
            paused: false,
            smooth: true,
            show_picker: false,
            picker_cursor: 0,
            picker_entries: vec![],
            picker_connected_idx: None,
            picker_scanning: false,
            last_error: None,
        }
    }

    fn push_to_buf(buf: &mut VecDeque<f64>, val: f64, max: usize) {
        buf.push_back(val);
        while buf.len() > max {
            buf.pop_front();
        }
    }

    pub fn push_eeg_packet(&mut self, _raw: &[u8]) {
        if self.paused {
            return;
        }
        #[cfg(feature = "local-decode")]
        let samples = if _raw.len() > 2 {
            let decoded = try_decode_eeg_12bit(&_raw[2..]);
            if decoded.is_empty() {
                vec![0.0; EEG_SAMPLES_PER_PACKET]
            } else {
                decoded
            }
        } else {
            vec![0.0; EEG_SAMPLES_PER_PACKET]
        };
        #[cfg(not(feature = "local-decode"))]
        let samples = vec![0.0; EEG_SAMPLES_PER_PACKET];
        for &v in &samples {
            Self::push_to_buf(&mut self.eeg_buf, v, BUF_SIZE);
        }
        self.total_packets += 1;
        let now = Instant::now();
        self.pkt_times.push_back(now);
        while self
            .pkt_times
            .front()
            .map(|t| now.duration_since(*t) > Duration::from_secs(2))
            .unwrap_or(false)
        {
            self.pkt_times.pop_front();
        }
    }

    pub fn push_eeg_samples(&mut self, samples: &[f64]) {
        if self.paused {
            return;
        }
        for &v in samples {
            Self::push_to_buf(&mut self.eeg_buf, v, BUF_SIZE);
        }
        self.total_packets += 1;
        let now = Instant::now();
        self.pkt_times.push_back(now);
        while self
            .pkt_times
            .front()
            .map(|t| now.duration_since(*t) > Duration::from_secs(2))
            .unwrap_or(false)
        {
            self.pkt_times.pop_front();
        }
    }

    pub fn push_accel(&mut self, x: f32, y: f32, z: f32) {
        if self.paused {
            return;
        }
        Self::push_to_buf(&mut self.accel_x, x as f64, IMU_BUF_SIZE);
        Self::push_to_buf(&mut self.accel_y, y as f64, IMU_BUF_SIZE);
        Self::push_to_buf(&mut self.accel_z, z as f64, IMU_BUF_SIZE);
        self.last_accel = Some((x, y, z));
    }

    pub fn push_gyro(&mut self, x: f32, y: f32, z: f32) {
        if self.paused {
            return;
        }
        Self::push_to_buf(&mut self.gyro_x, x as f64, IMU_BUF_SIZE);
        Self::push_to_buf(&mut self.gyro_y, y as f64, IMU_BUF_SIZE);
        Self::push_to_buf(&mut self.gyro_z, z as f64, IMU_BUF_SIZE);
        self.last_gyro = Some((x, y, z));
    }

    pub fn push_impedance(&mut self, kohms: f64) {
        self.last_impedance_kohms = Some(kohms);
        Self::push_to_buf(&mut self.imp_buf, kohms, IMP_BUF_SIZE);
    }

    pub fn clear(&mut self) {
        self.eeg_buf.clear();
        self.accel_x.clear();
        self.accel_y.clear();
        self.accel_z.clear();
        self.gyro_x.clear();
        self.gyro_y.clear();
        self.gyro_z.clear();
        self.imp_buf.clear();
        self.total_packets = 0;
        self.pkt_times.clear();
        self.battery = None;
        self.mac_address = None;
        self.firmware = None;
        self.hardware = None;
        self.last_accel = None;
        self.last_gyro = None;
        self.last_impedance_kohms = None;
        self.last_error = None;
    }

    fn pkt_rate(&self) -> f64 {
        let n = self.pkt_times.len();
        if n < 2 {
            return 0.0;
        }
        let span = self.pkt_times.back().unwrap().duration_since(self.pkt_times[0]).as_secs_f64();
        if span < 1e-9 { 0.0 } else { (n as f64 - 1.0) / span }
    }

    fn y_range(&self) -> f64 { Y_SCALES[self.scale_idx] }

    fn scale_up(&mut self) {
        if self.scale_idx + 1 < Y_SCALES.len() { self.scale_idx += 1; }
    }

    fn scale_down(&mut self) {
        if self.scale_idx > 0 { self.scale_idx -= 1; }
    }

    fn auto_scale(&mut self) {
        let peak = self.eeg_buf.iter().fold(0.0_f64, |acc, &v| acc.max(v.abs()));
        let needed = peak * 1.1;
        self.scale_idx = Y_SCALES.iter().position(|&s| s >= needed).unwrap_or(Y_SCALES.len() - 1);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn short_id(id: &str) -> String {
    let t = id.trim_matches(|c: char| c == '{' || c == '}');
    if t.len() > 8 { t[t.len() - 8..].to_uppercase() } else { t.to_uppercase() }
}

fn device_entry(d: &GuardianDevice) -> String {
    format!("{}  [{}]", d.name, short_id(&d.id))
}

fn smooth_signal(data: &[(f64, f64)], window: usize) -> Vec<(f64, f64)> {
    if data.len() < 3 || window < 2 { return data.to_vec(); }
    let half = window / 2;
    data.iter().enumerate().map(|(i, &(x, _))| {
        let start = i.saturating_sub(half);
        let end = (i + half + 1).min(data.len());
        let sum: f64 = data[start..end].iter().map(|&(_, y)| y).sum();
        (x, sum / (end - start) as f64)
    }).collect()
}

fn spinner_str() -> &'static str {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    SPINNER[(ms / 100) as usize % SPINNER.len()]
}

#[inline]
fn sep<'a>() -> Span<'a> { Span::styled(" │ ", Style::default().fg(Color::DarkGray)) }

#[inline]
fn key(s: &str) -> Span<'_> { Span::styled(s, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)) }

/// Build (x, y) chart data from a ring buffer, using `hz` as the x-axis rate.
fn buf_to_chart(buf: &VecDeque<f64>, hz: f64) -> Vec<(f64, f64)> {
    buf.iter().enumerate().map(|(i, &v)| (i as f64 / hz, v)).collect()
}

/// Auto-range a buffer for Y axis with 10% margin.
fn auto_range(buf: &VecDeque<f64>, default_min: f64, default_max: f64) -> (f64, f64) {
    if buf.is_empty() { return (default_min, default_max); }
    let min = buf.iter().copied().fold(f64::INFINITY, f64::min);
    let max = buf.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let margin = (max - min).max(0.01) * 0.1;
    (min - margin, max + margin)
}

// ── Simulator ─────────────────────────────────────────────────────────────────

fn sim_eeg(t: f64) -> f64 {
    let alpha = 20.0 * (2.0 * PI * 10.0 * t).sin();
    let beta = 6.0 * (2.0 * PI * 22.0 * t).sin();
    let theta = 10.0 * (2.0 * PI * 6.0 * t).sin();
    let delta = 4.0 * (2.0 * PI * 2.0 * t).sin();
    let noise = ((t * 1000.7).sin() * 9973.1).fract() * 8.0 - 4.0;
    let blink_phase = (t % 6.0) / 6.0;
    let blink = if blink_phase < 0.035 {
        let x = (blink_phase - 0.015) / 0.008;
        150.0 * (-x * x / 2.0).exp()
    } else { 0.0 };
    let clench_phase = (t % 10.3) / 10.3;
    let clench = if (0.40..0.44).contains(&clench_phase) {
        80.0 * (PI * (clench_phase - 0.40) / 0.04).sin() * (2.0 * PI * 65.0 * t).sin()
    } else { 0.0 };
    alpha + beta + theta + delta + noise + blink + clench
}

fn sim_impedance(t: f64) -> f64 {
    let base = 6.0 + 1.5 * (2.0 * PI * 0.02 * t).sin();
    let spike_ph = (t % 15.7) / 15.7;
    let spike = if (0.3..0.35).contains(&spike_ph) {
        25.0 * (-(((spike_ph - 0.325) / 0.012).powi(2)) / 2.0).exp()
    } else { 0.0 };
    (base + spike + 0.3 * ((t * 137.508).sin() * 7919.0).fract()).max(0.5)
}

fn spawn_simulator(app: Arc<Mutex<App>>) {
    tokio::spawn(async move {
        let eeg_interval = Duration::from_secs_f64(EEG_SAMPLES_PER_PACKET as f64 / EEG_HZ);
        let mut ticker = tokio::time::interval(eeg_interval);
        let dt = 1.0 / EEG_HZ;
        let mut t = 0.0_f64;
        let mut pkt = 0u64;
        loop {
            ticker.tick().await;
            let mut s = app.lock().unwrap();
            let t0 = t;
            t += EEG_SAMPLES_PER_PACKET as f64 * dt;
            if s.paused { continue; }

            // EEG
            let samples: Vec<f64> = (0..EEG_SAMPLES_PER_PACKET)
                .map(|i| sim_eeg(t0 + i as f64 * dt))
                .collect();
            s.push_eeg_samples(&samples);
            pkt += 1;

            // IMU (~52 Hz → every ~4 EEG packets)
            if pkt.is_multiple_of(4) {
                let ax = (0.01 * (2.0 * PI * 0.3 * t).sin()) as f32;
                let ay = (0.02 * (2.0 * PI * 0.5 * t).cos()) as f32;
                let az = (-1.0 + 0.005 * (2.0 * PI * 0.1 * t).sin()) as f32;
                s.push_accel(ax, ay, az);

                let gx = (2.5 * (2.0 * PI * 0.2 * t).sin()) as f32;
                let gy = (1.8 * (2.0 * PI * 0.3 * t).cos()) as f32;
                let gz = (0.7 * (2.0 * PI * 0.15 * t).sin()) as f32;
                s.push_gyro(gx, gy, gz);
            }

            // Impedance (~every 500ms → every 6 packets)
            if pkt.is_multiple_of(6) { s.push_impedance(sim_impedance(t)); }

            // Battery drain
            if pkt.is_multiple_of(750) {
                s.battery = Some((92.0 - t / 60.0).clamp(0.0, 100.0) as u8);
            }
        }
    });
}

// ── BLE helpers ───────────────────────────────────────────────────────────────

struct ScanResult { devices: Vec<GuardianDevice>, error: Option<String> }

fn start_scan(config: GuardianClientConfig) -> oneshot::Receiver<ScanResult> {
    let (tx, rx) = oneshot::channel();
    let deadline = Duration::from_secs(config.scan_timeout_secs + 10);
    tokio::spawn(async move {
        let result = match tokio::time::timeout(deadline, GuardianClient::new(config).scan_all()).await {
            Ok(Ok(devices)) => ScanResult { devices, error: None },
            Ok(Err(e)) => ScanResult { devices: vec![], error: Some(format!("{e}")) },
            Err(_) => ScanResult { devices: vec![], error: Some("scan timed out".into()) },
        };
        let _ = tx.send(result);
    });
    rx
}

fn restart_scan(app: &Arc<Mutex<App>>, pending_scan: &mut Option<oneshot::Receiver<ScanResult>>, retry_at: &mut Option<tokio::time::Instant>, delay_secs: u64) {
    { let mut s = app.lock().unwrap(); s.clear(); s.picker_connected_idx = None; s.picker_entries.clear(); s.show_picker = false; s.mode = AppMode::Scanning; s.picker_scanning = true; }
    if pending_scan.is_none() { *retry_at = Some(tokio::time::Instant::now() + Duration::from_secs(delay_secs)); }
}

fn spawn_event_task(mut rx: mpsc::Receiver<GuardianEvent>, app: Arc<Mutex<App>>) {
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let mut s = app.lock().unwrap();
            match ev {
                GuardianEvent::Connected(_) => {}
                GuardianEvent::Disconnected => { s.mode = AppMode::Disconnected; s.picker_connected_idx = None; break; }
                GuardianEvent::Eeg(r) => { s.push_eeg_packet(&r.raw_data); }
                GuardianEvent::Accelerometer(r) => { s.push_accel(r.sample.x, r.sample.y, r.sample.z); }
                GuardianEvent::Gyroscope(r) => { s.push_gyro(r.sample.x, r.sample.y, r.sample.z); }
                GuardianEvent::Impedance(r) => { s.push_impedance(r.impedance_kohms); }
                GuardianEvent::Battery(b) => { s.battery = Some(b.level); }
                GuardianEvent::DeviceInfo(info) => { s.mac_address = Some(info.mac_address); s.firmware = Some(info.firmware_version); s.hardware = Some(info.hardware_version); }
            }
        }
    });
}

struct ConnectOutcome { rx: mpsc::Receiver<GuardianEvent>, handle: GuardianHandle, device_idx: usize, name: String, id: String }

fn start_connect(idx: usize, device: GuardianDevice, app: Arc<Mutex<App>>, cfg: GuardianClientConfig) -> oneshot::Receiver<Option<ConnectOutcome>> {
    let (tx, rx) = oneshot::channel();
    { let mut s = app.lock().unwrap(); s.clear(); s.mode = AppMode::Connecting(device.name.clone()); s.picker_connected_idx = None; s.show_picker = false; }
    tokio::spawn(async move {
        let client = GuardianClient::new(cfg);
        match client.connect_to(device.clone()).await {
            Ok((evt_rx, h)) => match h.start_recording().await {
                Ok(()) => { let _ = tx.send(Some(ConnectOutcome { rx: evt_rx, handle: h, device_idx: idx, name: device.name.clone(), id: short_id(&device.id) })); }
                Err(e) => { h.disconnect().await.ok(); let mut s = app.lock().unwrap(); s.mode = AppMode::Disconnected; s.last_error = Some(format!("start failed: {e}")); let _ = tx.send(None); }
            },
            Err(e) => { let mut s = app.lock().unwrap(); s.mode = AppMode::Disconnected; s.last_error = Some(format!("connect failed: {e}")); let _ = tx.send(None); }
        }
    });
    rx
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let root = Layout::vertical([Constraint::Length(3), Constraint::Min(0), Constraint::Length(3)]).split(area);
    draw_header(frame, root[0], app);
    match app.view {
        ViewMode::Eeg => draw_eeg_full(frame, root[1], app),
        ViewMode::Imu => draw_imu_full(frame, root[1], app),
        ViewMode::Impedance => draw_impedance_full(frame, root[1], app),
        ViewMode::All => draw_all(frame, root[1], app),
    }
    draw_footer(frame, root[2], app);
    if app.show_picker { draw_device_picker(frame, area, app); }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let (label, color) = match &app.mode {
        AppMode::Scanning => (format!("{} Scanning…", spinner_str()), Color::Yellow),
        AppMode::Connecting(n) => (format!("{} Connecting to {n}…", spinner_str()), Color::Yellow),
        AppMode::Connected { name, id } => (format!("● {name}  [{id}]"), Color::Green),
        AppMode::Simulated => ("◆ Simulated".into(), Color::Cyan),
        AppMode::NoDevices => (format!("{} No devices — retrying…", spinner_str()), Color::Yellow),
        AppMode::Disconnected => { let r = app.last_error.as_deref().map(|e| format!(" ({e})")).unwrap_or_default(); (format!("{} Disconnected{r} — retrying…", spinner_str()), Color::Red) }
    };
    let bat = app.battery.map(|b| format!("Bat {b}%")).unwrap_or("Bat N/A".into());
    let imp = app.last_impedance_kohms.map(|k| format!("{k:.1}kΩ")).unwrap_or("N/A".into());
    let view_label = match app.view { ViewMode::Eeg => "EEG", ViewMode::Imu => "IMU", ViewMode::Impedance => "IMP", ViewMode::All => "ALL" };
    let line = Line::from(vec![
        Span::styled(" IDUN Guardian ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)), sep(),
        Span::styled(label, Style::default().fg(color).add_modifier(Modifier::BOLD)), sep(),
        Span::styled(view_label, Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD)), sep(),
        Span::styled(bat, Style::default().fg(Color::White)), sep(),
        Span::styled(format!("Z:{imp}"), Style::default().fg(Color::LightCyan)), sep(),
        Span::styled(format!("{:.1}pkt/s", app.pkt_rate()), Style::default().fg(Color::White)),
        Span::raw(" "),
    ]);
    frame.render_widget(Paragraph::new(line).block(Block::default().borders(Borders::ALL)), area);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let pause_span = if app.paused { Span::styled("  ⏸ PAUSED", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)) } else { Span::raw("") };
    let keys = Line::from(vec![
        Span::raw(" "), key("[Tab]"), Span::raw("Pick "), key("[1]"), Span::raw("EEG "),
        key("[2]"), Span::raw("IMU "), key("[3]"), Span::raw("Imp "), key("[4]"), Span::raw("All "),
        key("[+/-]"), Span::raw("Scale "), key("[a]"), Span::raw("Auto "), key("[v]"), Span::raw("Smooth "),
        key("[p]"), Span::raw("Pause "), key("[r]"), Span::raw("Resume "), key("[z]"), Span::raw("Z-stream "),
        key("[c]"), Span::raw("Clear "), key("[q]"), Span::raw("Quit"), pause_span,
    ]);
    let mac = app.mac_address.as_deref().unwrap_or("N/A");
    let fw = app.firmware.as_deref().unwrap_or("N/A");
    let hw = app.hardware.as_deref().unwrap_or("N/A");
    let (ax, ay, az) = app.last_accel.unwrap_or((0.0, 0.0, 0.0));
    let (gx, gy, gz) = app.last_gyro.unwrap_or((0.0, 0.0, 0.0));
    let info_line = Line::from(vec![
        Span::raw(" "), Span::styled("MAC ", Style::default().fg(Color::DarkGray)), Span::styled(mac, Style::default().fg(Color::Cyan)),
        Span::raw("  "), Span::styled("FW ", Style::default().fg(Color::DarkGray)), Span::styled(fw, Style::default().fg(Color::Yellow)),
        Span::raw("  "), Span::styled("HW ", Style::default().fg(Color::DarkGray)), Span::styled(hw, Style::default().fg(Color::Magenta)),
        Span::raw("  "), Span::styled(format!("A:{ax:+.2},{ay:+.2},{az:+.2}g"), Style::default().fg(Color::DarkGray)),
        Span::raw(" "), Span::styled(format!("G:{gx:+.1},{gy:+.1},{gz:+.1}°/s"), Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(vec![keys, info_line]).block(Block::default().borders(Borders::ALL)), area);
}

// ── Chart builders ────────────────────────────────────────────────────────────

fn draw_eeg_full(frame: &mut Frame, area: Rect, app: &App) {
    draw_eeg_chart(frame, area, app);
}

fn draw_imu_full(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).split(area);
    draw_accel_chart(frame, rows[0], app);
    draw_gyro_chart(frame, rows[1], app);
}

fn draw_impedance_full(frame: &mut Frame, area: Rect, app: &App) {
    draw_impedance_chart(frame, area, app);
}

fn draw_all(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Ratio(2, 5), // EEG (biggest)
        Constraint::Ratio(1, 5), // Accel
        Constraint::Ratio(1, 5), // Gyro
        Constraint::Ratio(1, 5), // Impedance
    ]).split(area);
    draw_eeg_chart(frame, rows[0], app);
    draw_accel_chart(frame, rows[1], app);
    draw_gyro_chart(frame, rows[2], app);
    draw_impedance_chart(frame, rows[3], app);
}

fn draw_eeg_chart(frame: &mut Frame, area: Rect, app: &App) {
    let yr = app.y_range();
    let color = Color::Cyan;
    let data: Vec<(f64, f64)> = app.eeg_buf.iter().enumerate().map(|(i, &v)| (i as f64 / EEG_HZ, v.clamp(-yr, yr))).collect();
    let smoothed = if app.smooth { smooth_signal(&data, SMOOTH_WINDOW) } else { vec![] };
    let datasets: Vec<Dataset> = if app.smooth && !data.is_empty() {
        vec![
            Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(Color::Rgb(0, 90, 110))).data(&data),
            Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(color)).data(&smoothed),
        ]
    } else {
        vec![Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(color)).data(&data)]
    };
    let (mn, mx, rms) = buf_stats(&app.eeg_buf);
    let title = format!(" EEG  min:{mn:+.0} max:{mx:+.0} rms:{rms:.0} µV  ±{yr:.0}µV ");
    let chart = Chart::new(datasets)
        .block(Block::default().title(Span::styled(title, Style::default().fg(color).add_modifier(Modifier::BOLD))).borders(Borders::ALL).border_style(Style::default().fg(color)))
        .x_axis(Axis::default().bounds([0.0, WINDOW_SECS]).labels(vec!["0s".into(), format!("{:.0}s", WINDOW_SECS)]).style(Style::default().fg(Color::DarkGray)))
        .y_axis(Axis::default().bounds([-yr, yr]).labels(vec![format!("{:+.0}", -yr), "0".into(), format!("{:+.0}", yr)]).style(Style::default().fg(Color::DarkGray)));
    frame.render_widget(chart, area);
}

fn draw_accel_chart(frame: &mut Frame, area: Rect, app: &App) {
    let (xmin, xmax) = auto_range(&app.accel_x, -2.0, 2.0);
    let (ymin, ymax) = auto_range(&app.accel_y, -2.0, 2.0);
    let (zmin, zmax) = auto_range(&app.accel_z, -2.0, 2.0);
    let lo = xmin.min(ymin).min(zmin);
    let hi = xmax.max(ymax).max(zmax);
    let dx: Vec<(f64, f64)> = buf_to_chart(&app.accel_x, IMU_HZ);
    let dy: Vec<(f64, f64)> = buf_to_chart(&app.accel_y, IMU_HZ);
    let dz: Vec<(f64, f64)> = buf_to_chart(&app.accel_z, IMU_HZ);
    let (ax, ay, az) = app.last_accel.unwrap_or((0.0, 0.0, 0.0));
    let title = format!(" Accel  x:{ax:+.3}g  y:{ay:+.3}g  z:{az:+.3}g ");
    let datasets = vec![
        Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(Color::Red)).data(&dx),
        Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(Color::Green)).data(&dy),
        Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(Color::Blue)).data(&dz),
    ];
    let chart = Chart::new(datasets)
        .block(Block::default().title(Span::styled(title, Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD))).borders(Borders::ALL).border_style(Style::default().fg(Color::LightYellow)))
        .x_axis(Axis::default().bounds([0.0, WINDOW_SECS]).labels(vec!["0s".into(), format!("{:.0}s", WINDOW_SECS)]).style(Style::default().fg(Color::DarkGray)))
        .y_axis(Axis::default().bounds([lo, hi]).labels(vec![format!("{lo:.2}"), format!("{:.2}g", (lo+hi)/2.0), format!("{hi:.2}")]).style(Style::default().fg(Color::DarkGray)));
    frame.render_widget(chart, area);
}

fn draw_gyro_chart(frame: &mut Frame, area: Rect, app: &App) {
    let (xmin, xmax) = auto_range(&app.gyro_x, -10.0, 10.0);
    let (ymin, ymax) = auto_range(&app.gyro_y, -10.0, 10.0);
    let (zmin, zmax) = auto_range(&app.gyro_z, -10.0, 10.0);
    let lo = xmin.min(ymin).min(zmin);
    let hi = xmax.max(ymax).max(zmax);
    let dx: Vec<(f64, f64)> = buf_to_chart(&app.gyro_x, IMU_HZ);
    let dy: Vec<(f64, f64)> = buf_to_chart(&app.gyro_y, IMU_HZ);
    let dz: Vec<(f64, f64)> = buf_to_chart(&app.gyro_z, IMU_HZ);
    let (gx, gy, gz) = app.last_gyro.unwrap_or((0.0, 0.0, 0.0));
    let title = format!(" Gyro  x:{gx:+.1}°/s  y:{gy:+.1}°/s  z:{gz:+.1}°/s ");
    let datasets = vec![
        Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(Color::Red)).data(&dx),
        Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(Color::Green)).data(&dy),
        Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(Color::Blue)).data(&dz),
    ];
    let chart = Chart::new(datasets)
        .block(Block::default().title(Span::styled(title, Style::default().fg(Color::LightMagenta).add_modifier(Modifier::BOLD))).borders(Borders::ALL).border_style(Style::default().fg(Color::LightMagenta)))
        .x_axis(Axis::default().bounds([0.0, WINDOW_SECS]).labels(vec!["0s".into(), format!("{:.0}s", WINDOW_SECS)]).style(Style::default().fg(Color::DarkGray)))
        .y_axis(Axis::default().bounds([lo, hi]).labels(vec![format!("{lo:.1}"), format!("{:.1}°/s", (lo+hi)/2.0), format!("{hi:.1}")]).style(Style::default().fg(Color::DarkGray)));
    frame.render_widget(chart, area);
}

fn draw_impedance_chart(frame: &mut Frame, area: Rect, app: &App) {
    let color = Color::LightGreen;
    let (lo, hi) = auto_range(&app.imp_buf, 0.0, 20.0);
    let data: Vec<(f64, f64)> = app.imp_buf.iter().enumerate().map(|(i, &v)| (i as f64, v)).collect();
    let cur = app.last_impedance_kohms.map(|k| format!("{k:.2}")).unwrap_or("N/A".into());
    let title = format!(" Impedance  {cur} kΩ  ({} pts) ", app.imp_buf.len());
    let xmax = (app.imp_buf.len() as f64).max(10.0);
    let chart = Chart::new(vec![Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(color)).data(&data)])
        .block(Block::default().title(Span::styled(title, Style::default().fg(color).add_modifier(Modifier::BOLD))).borders(Borders::ALL).border_style(Style::default().fg(color)))
        .x_axis(Axis::default().bounds([0.0, xmax]).labels(vec!["oldest".to_string(), "newest".to_string()]).style(Style::default().fg(Color::DarkGray)))
        .y_axis(Axis::default().bounds([lo, hi]).labels(vec![format!("{lo:.1}"), format!("{:.1}kΩ", (lo+hi)/2.0), format!("{hi:.1}")]).style(Style::default().fg(Color::DarkGray)));
    frame.render_widget(chart, area);
}

fn buf_stats(buf: &VecDeque<f64>) -> (f64, f64, f64) {
    if buf.is_empty() { return (0.0, 0.0, 0.0); }
    let mn = buf.iter().copied().fold(f64::INFINITY, f64::min);
    let mx = buf.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let rms = (buf.iter().map(|v| v*v).sum::<f64>() / buf.len() as f64).sqrt();
    (mn, mx, rms)
}

// ── Device picker ─────────────────────────────────────────────────────────────

fn draw_device_picker(frame: &mut Frame, area: Rect, app: &App) {
    let n = app.picker_entries.len().max(1);
    let bh = (n as u16 + 6).min(area.height);
    let bw = (area.width * 60 / 100).max(52).min(area.width);
    let x = area.x + area.width.saturating_sub(bw) / 2;
    let y = area.y + area.height.saturating_sub(bh) / 2;
    let popup = Rect::new(x, y, bw, bh);
    frame.render_widget(Clear, popup);
    let title = if app.picker_scanning { format!(" {} Scanning… ({} found) ", spinner_str(), app.picker_entries.len()) }
                else { format!(" Select Device ({} found) ", app.picker_entries.len()) };
    frame.render_widget(Block::default().title(Span::styled(title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD))).borders(Borders::ALL), popup);
    let inner = popup.inner(Margin { horizontal: 1, vertical: 1 });
    let [list_area, _, hint_area] = Layout::vertical([Constraint::Length(inner.height.saturating_sub(3)), Constraint::Length(1), Constraint::Length(2)]).areas(inner);
    let items: Vec<ListItem> = if app.picker_entries.is_empty() {
        vec![ListItem::new(Span::styled("  No devices — press [s] to scan", Style::default().fg(Color::DarkGray)))]
    } else {
        app.picker_entries.iter().enumerate().map(|(i, e)| {
            let conn = app.picker_connected_idx == Some(i);
            let (b, c, sfx) = if conn { ("● ", Color::Green, " ← connected") } else { ("  ", Color::White, "") };
            ListItem::new(Span::styled(format!("{b}{e}{sfx}"), Style::default().fg(c)))
        }).collect()
    };
    let mut ls = ListState::default();
    if !app.picker_entries.is_empty() { ls.select(Some(app.picker_cursor)); }
    frame.render_stateful_widget(List::new(items).highlight_style(Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)).highlight_symbol("▶ "), list_area, &mut ls);
    frame.render_widget(Paragraph::new(vec![
        Line::from(vec![key(" [↑↓]"), Span::raw(" Nav "), key("[↵]"), Span::raw(" Connect "), key("[s]"), Span::raw(" Rescan "), key("[Esc]"), Span::raw(" Close")]),
    ]), hint_area);
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    use std::io::IsTerminal as _;
    if !io::stdout().is_terminal() { eprintln!("Error: idun-tui requires a real terminal."); std::process::exit(1); }
    { use std::fs::File; let p = std::env::temp_dir().join("idun-tui.log"); if let Ok(f) = File::create(&p) { env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).target(env_logger::Target::Pipe(Box::new(f))).init(); log::info!("Logging to {}", p.display()); } }

    let simulate = std::env::args().any(|a| a == "--simulate");
    #[cfg(not(feature = "simulate"))]
    if simulate {
        eprintln!("Error: --simulate requires the `simulate` feature.");
        eprintln!("  cargo run --bin idun-tui --features simulate -- --simulate");
        std::process::exit(1);
    }
    let mains_60hz = std::env::args().any(|a| a == "--60hz");
    let app = Arc::new(Mutex::new(App::new()));
    let mut devices: Vec<GuardianDevice> = vec![];
    let mut handle: Option<Arc<GuardianHandle>> = None;
    let mut pending_scan: Option<oneshot::Receiver<ScanResult>> = None;
    let mut pending_connect: Option<oneshot::Receiver<Option<ConnectOutcome>>> = None;
    let mut retry_at: Option<tokio::time::Instant> = None;
    let scan_cfg = GuardianClientConfig { mains_freq_60hz: mains_60hz, scan_timeout_secs: 5, name_prefix: "IGE".into(), api_token: None };

    if simulate {
        let mut s = app.lock().unwrap();
        s.mode = AppMode::Simulated; s.scale_idx = 4; s.view = ViewMode::All;
        s.mac_address = Some("SIM-00-11-22-33-44-55".into());
        s.firmware = Some("sim-1.0.0".into()); s.hardware = Some("sim-3.0a".into()); s.battery = Some(92);
        drop(s);
        spawn_simulator(Arc::clone(&app));
    } else {
        app.lock().unwrap().picker_scanning = true;
        pending_scan = Some(start_scan(scan_cfg.clone()));
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let tick = Duration::from_millis(33);

    'main: loop {
        if let Some(ref mut rx) = pending_scan { if let Ok(sr) = rx.try_recv() { pending_scan = None; devices = sr.devices; { let mut s = app.lock().unwrap(); s.picker_entries = devices.iter().map(device_entry).collect(); s.picker_scanning = false; if devices.is_empty() { s.mode = AppMode::NoDevices; if let Some(e) = sr.error { s.last_error = Some(e); } } } if devices.is_empty() { retry_at = Some(tokio::time::Instant::now() + Duration::from_secs(RETRY_SECS)); } else if handle.is_none() && pending_connect.is_none() { pending_connect = Some(start_connect(0, devices[0].clone(), Arc::clone(&app), scan_cfg.clone())); } } }
        if let Some(ref mut rx) = pending_connect { if let Ok(result) = rx.try_recv() { pending_connect = None; if let Some(o) = result { let h = Arc::new(o.handle); { let mut s = app.lock().unwrap(); s.mode = AppMode::Connected { name: o.name, id: o.id }; s.last_error = None; s.picker_connected_idx = Some(o.device_idx); } handle = Some(Arc::clone(&h)); spawn_event_task(o.rx, Arc::clone(&app)); } else { devices.clear(); restart_scan(&app, &mut pending_scan, &mut retry_at, RECONNECT_DELAY_SECS); } } }
        if !simulate { let disc = matches!(app.lock().unwrap().mode, AppMode::Disconnected); if disc && handle.is_some() { handle = None; devices.clear(); restart_scan(&app, &mut pending_scan, &mut retry_at, RECONNECT_DELAY_SECS); } }
        if let Some(when) = retry_at { if tokio::time::Instant::now() >= when && pending_scan.is_none() && pending_connect.is_none() { retry_at = None; { let mut s = app.lock().unwrap(); s.picker_scanning = true; s.mode = AppMode::Scanning; } pending_scan = Some(start_scan(scan_cfg.clone())); } }

        { let s = app.lock().unwrap(); terminal.draw(|f| draw(f, &s))?; }

        if event::poll(tick)? {
            if let Event::Key(ke) = event::read()? {
                if ke.modifiers.contains(KeyModifiers::CONTROL) && ke.code == KeyCode::Char('c') { break 'main; }
                let mut s = app.lock().unwrap();
                if s.show_picker {
                    match ke.code {
                        KeyCode::Esc => s.show_picker = false,
                        KeyCode::Up => { if s.picker_cursor > 0 { s.picker_cursor -= 1; } }
                        KeyCode::Down => { if !s.picker_entries.is_empty() && s.picker_cursor + 1 < s.picker_entries.len() { s.picker_cursor += 1; } }
                        KeyCode::Char('s') => { if pending_scan.is_none() { s.picker_scanning = true; drop(s); pending_scan = Some(start_scan(scan_cfg.clone())); continue; } }
                        KeyCode::Enter => { if !devices.is_empty() { let idx = s.picker_cursor.min(devices.len()-1); let dev = devices[idx].clone(); drop(s); if let Some(h) = handle.take() { h.disconnect().await.ok(); } pending_connect = Some(start_connect(idx, dev, Arc::clone(&app), scan_cfg.clone())); continue; } }
                        _ => {}
                    }
                    continue;
                }
                match ke.code {
                    KeyCode::Char('q') | KeyCode::Esc => break 'main,
                    KeyCode::Tab => s.show_picker = true,
                    KeyCode::Char('1') => s.view = ViewMode::Eeg,
                    KeyCode::Char('2') => s.view = ViewMode::Imu,
                    KeyCode::Char('3') => s.view = ViewMode::Impedance,
                    KeyCode::Char('4') => s.view = ViewMode::All,
                    KeyCode::Char('+') | KeyCode::Char('=') => s.scale_up(),
                    KeyCode::Char('-') => s.scale_down(),
                    KeyCode::Char('a') => s.auto_scale(),
                    KeyCode::Char('v') => s.smooth = !s.smooth,
                    KeyCode::Char('c') => s.clear(),
                    KeyCode::Char('p') => { s.paused = true; if let Some(ref h) = handle { let h = Arc::clone(h); drop(s); h.stop_recording().await.ok(); continue; } }
                    KeyCode::Char('r') => { s.paused = false; if let Some(ref h) = handle { let h = Arc::clone(h); drop(s); h.start_recording().await.ok(); continue; } }
                    KeyCode::Char('z') => { if let Some(ref h) = handle { let h = Arc::clone(h); s.view = ViewMode::Impedance; drop(s); h.stop_recording().await.ok(); h.start_impedance().await.ok(); continue; } else { s.view = ViewMode::Impedance; } }
                    KeyCode::Char('s') => { if pending_scan.is_none() { s.picker_scanning = true; drop(s); pending_scan = Some(start_scan(scan_cfg.clone())); continue; } }
                    KeyCode::Char('d') => { if let Some(h) = handle.take() { drop(s); h.disconnect().await.ok(); let mut s2 = app.lock().unwrap(); s2.mode = AppMode::Disconnected; s2.picker_connected_idx = None; continue; } }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    if let Some(h) = handle.take() { h.stop_recording().await.ok(); h.disconnect().await.ok(); }
    Ok(())
}
