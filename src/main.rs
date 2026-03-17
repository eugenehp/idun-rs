use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::Result;
use log::{error, info, warn};

use idun::cloud::CloudDecoder;
use idun::guardian_client::{GuardianClient, GuardianClientConfig};
#[cfg(feature = "local-decode")]
use idun::parse::{parse_eeg_packet, try_decode_eeg_12bit};
use idun::types::{DecodeSource, GuardianEvent};

/// Heuristic: check if decoded samples look like valid EEG.
///
/// Returns `true` if the samples are plausibly valid:
/// - Non-empty
/// - Not all zeros
/// - RMS is in a plausible range (0.1–2000 µV)
#[cfg(feature = "local-decode")]
fn samples_look_valid(samples: &[f64]) -> bool {
    if samples.is_empty() {
        return false;
    }
    let all_zero = samples.iter().all(|&v| v == 0.0);
    if all_zero {
        return false;
    }
    let rms = (samples.iter().map(|v| v * v).sum::<f64>() / samples.len() as f64).sqrt();
    // Plausible EEG range: 0.1–2000 µV RMS
    (0.1..=2000.0).contains(&rms)
}

fn print_usage() {
    eprintln!("Usage: idun [OPTIONS]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --impedance     Stream impedance instead of EEG");
    eprintln!("  --60hz          Use 60 Hz notch filter (default: 50 Hz)");
    eprintln!("  --address ADDR  Connect to a specific BLE address");
    eprintln!("  --csv FILE      Write data to a CSV file");
    eprintln!("  --timeout SECS  Scan timeout in seconds (default: 15)");
    eprintln!("  --decode        Attempt experimental 12-bit EEG decoding");
    eprintln!("  --cloud         Enable IDUN Cloud fallback for EEG decoding");
    eprintln!("                  (requires IDUN_API_TOKEN env var or --token)");
    eprintln!("  --token TOKEN   IDUN API token (alternative to IDUN_API_TOKEN env var)");
    eprintln!("  --help          Show this help");
    eprintln!();
    eprintln!("Decoding priority (when --decode --cloud are both set):");
    eprintln!("  1. Try local 12-bit decoder");
    eprintln!("  2. If local decode fails → send to IDUN Cloud for decoding");
    eprintln!("  3. If cloud unavailable → output raw packet only");
    eprintln!();
    eprintln!("Interactive commands (type + Enter while streaming):");
    eprintln!("  q  quit    b  battery    z  impedance    s  stop    m  start    d  disconnect");
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return Ok(());
    }

    let mains_60hz = args.iter().any(|a| a == "--60hz");
    let impedance_mode = args.iter().any(|a| a == "--impedance");
    let use_cloud = args.iter().any(|a| a == "--cloud");

    #[cfg(feature = "local-decode")]
    let decode_eeg = args.iter().any(|a| a == "--decode");
    #[cfg(not(feature = "local-decode"))]
    let decode_eeg = {
        if args.iter().any(|a| a == "--decode") {
            warn!("--decode flag ignored: compile with `local-decode` feature to enable experimental EEG/IMU decoding");
        }
        false
    };

    let address = args
        .windows(2)
        .find(|w| w[0] == "--address")
        .map(|w| w[1].clone());

    let api_token = args
        .windows(2)
        .find(|w| w[0] == "--token")
        .map(|w| w[1].clone());

    let csv_path = args
        .windows(2)
        .find(|w| w[0] == "--csv")
        .map(|w| PathBuf::from(&w[1]));

    let timeout: u64 = args
        .windows(2)
        .find(|w| w[0] == "--timeout")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(15);

    let config = GuardianClientConfig {
        mains_freq_60hz: mains_60hz,
        scan_timeout_secs: timeout,
        name_prefix: "IGE".into(),
        api_token: None,
    };

    // ── Connect to Guardian earbud ────────────────────────────────────────
    let client = GuardianClient::new(config);

    info!("Connecting to Guardian earbud …");
    let (mut rx, handle) = if let Some(addr) = &address {
        info!("Using specified address: {addr}");
        let devices = client.scan_all().await?;
        let device = devices
            .into_iter()
            .find(|d| d.id.contains(addr.as_str()) || d.name.contains(addr.as_str()))
            .ok_or_else(|| anyhow::anyhow!("No device found matching '{addr}'"))?;
        info!("Found matching device: {} [{}]", device.name, device.id);
        client.connect_to(device).await?
    } else {
        client.connect().await?
    };

    let handle = std::sync::Arc::new(handle);
    let device_mac = handle.mac_address.clone();

    // ── Set up cloud decoder (if enabled) ─────────────────────────────────
    let mut cloud_decoder: Option<CloudDecoder> = None;
    let mut cloud_failed = false; // sticky flag: don't retry after first failure
    #[cfg(feature = "local-decode")]
    let mut local_fail_count: u64 = 0;

    if use_cloud {
        // Resolve API token: --token flag > IDUN_API_TOKEN env var
        let token = if let Some(t) = api_token {
            t
        } else {
            match std::env::var("IDUN_API_TOKEN") {
                Ok(t) => t,
                Err(_) => {
                    error!("--cloud requires an API token. Set IDUN_API_TOKEN or use --token");
                    std::process::exit(1);
                }
            }
        };

        info!("Cloud fallback enabled — will connect to IDUN Cloud if local decoding fails");
        // Don't connect immediately; connect lazily on first local decode failure
        cloud_decoder = Some(CloudDecoder::new(token, device_mac.clone()));
    }

    // ── CSV output ────────────────────────────────────────────────────────
    let mut csv_writer: Option<std::io::BufWriter<std::fs::File>> = if let Some(ref path) = csv_path
    {
        let file = std::fs::File::create(path)?;
        let mut writer = std::io::BufWriter::new(file);
        if impedance_mode {
            writeln!(writer, "timestamp_ms,impedance_ohms,impedance_kohms")?;
        } else if decode_eeg || use_cloud {
            writeln!(writer, "timestamp_ms,packet_index,sample_index,uv,source")?;
        } else {
            writeln!(writer, "timestamp_ms,packet_index,raw_base64")?;
        }
        info!("Writing CSV to: {}", path.display());
        Some(writer)
    } else {
        None
    };

    // ── Start streaming ───────────────────────────────────────────────────
    if impedance_mode {
        handle.start_impedance().await?;
        info!("Impedance streaming started. Press Ctrl-C or type 'q' + Enter to quit.\n");
    } else {
        handle.start_recording().await?;
        info!("EEG streaming started. Press Ctrl-C or type 'q' + Enter to quit.\n");
    }

    if decode_eeg && use_cloud {
        info!("Decode chain: local 12-bit → cloud fallback → raw");
    } else if decode_eeg {
        info!("Decode mode: local 12-bit only (experimental)");
    } else if use_cloud {
        info!("Decode mode: cloud only");
    } else {
        info!("Decode mode: raw packets only (use --decode and/or --cloud to decode)");
    }

    info!("Commands (type + Enter):");
    info!("  q  – quit       b  – battery    z  – impedance");
    info!("  s  – stop       m  – start      d  – disconnect\n");

    // ── Stdin command loop ────────────────────────────────────────────────
    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    std::thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) => {
                    if line_tx.send(l.trim().to_owned()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let handle_cmd = std::sync::Arc::clone(&handle);
    tokio::spawn(async move {
        while let Some(line) = line_rx.recv().await {
            if line.is_empty() {
                continue;
            }
            match line.as_str() {
                "q" => {
                    info!("Quit requested.");
                    handle_cmd.disconnect().await.ok();
                    std::process::exit(0);
                }
                "b" => match handle_cmd.read_battery().await {
                    Ok(level) => info!("Battery: {level}%"),
                    Err(e) => error!("Battery read error: {e}"),
                },
                "z" => {
                    info!("Stopping EEG, starting impedance…");
                    handle_cmd.stop_recording().await.ok();
                    if let Err(e) = handle_cmd.start_impedance().await {
                        error!("Impedance start error: {e}");
                    }
                }
                "s" => {
                    info!("Stopping measurement…");
                    if let Err(e) = handle_cmd.stop_recording().await {
                        error!("Stop error: {e}");
                    }
                }
                "m" => {
                    info!("Starting measurement…");
                    if let Err(e) = handle_cmd.start_recording().await {
                        error!("Start error: {e}");
                    }
                }
                "d" => {
                    info!("Disconnecting…");
                    handle_cmd.disconnect().await.ok();
                    std::process::exit(0);
                }
                _ => {
                    info!("Unknown command: '{line}'. Valid: q b z s m d");
                }
            }
        }
    });

    // ── Also drain cloud decoded events in the background ─────────────────
    // (Cloud responses arrive asynchronously; we print them as they come)

    // ── Main event loop ───────────────────────────────────────────────────
    let mut eeg_count: u64 = 0;
    let mut cloud_seq: u64 = 0;

    while let Some(event) = rx.recv().await {
        // ── Check for cloud decoded responses ────────────────────────
        if let Some(ref mut cd) = cloud_decoder {
            while let Ok(Some(decoded)) = cd.try_recv_decoded() {
                println!(
                    "[CLOUD] Decoded response: action={} seq={:?}",
                    decoded.action,
                    decoded.sequence,
                );
                // TODO: extract actual sample values from decoded.data
                // The exact JSON structure depends on the IDUN Cloud API version
            }
        }

        match event {
            GuardianEvent::Connected(name) => {
                info!("✅  Connected to: {name}");
            }
            GuardianEvent::Disconnected => {
                info!("❌  Disconnected from device.");
                break;
            }
            GuardianEvent::Eeg(reading) => {
                eeg_count += 1;

                if decode_eeg || use_cloud {
                    // ── Step 1: Try local decoding ────────────────────
                    #[allow(unused_mut)]
                    let mut samples: Option<Vec<f64>> = None;
                    let mut source = DecodeSource::None;

                    #[cfg(feature = "local-decode")]
                    if decode_eeg {
                        if let Some(header) = parse_eeg_packet(&reading.raw_data) {
                            let decoded = try_decode_eeg_12bit(&header.payload);
                            if samples_look_valid(&decoded) {
                                samples = Some(decoded);
                                source = DecodeSource::Local;
                            } else {
                                local_fail_count += 1;
                                if local_fail_count <= 3 || local_fail_count % 100 == 0 {
                                    warn!(
                                        "[DECODE] Local decode failed (#{local_fail_count}): \
                                         {} samples, looks invalid",
                                        decoded.len()
                                    );
                                }
                            }
                        }
                    }

                    // ── Step 2: Cloud fallback ────────────────────────
                    if samples.is_none() && use_cloud && !cloud_failed {
                        if let Some(ref mut cd) = cloud_decoder {
                            // Lazy connect: only connect to cloud on first failure
                            if !cd.is_connected() {
                                info!("[CLOUD] Local decode failed — connecting to IDUN Cloud…");
                                match cd.connect().await {
                                    Ok(()) => {
                                        info!("[CLOUD] Connected! Falling back to cloud decoding.");
                                    }
                                    Err(e) => {
                                        error!("[CLOUD] Failed to connect: {e}");
                                        error!("[CLOUD] Cloud fallback disabled for this session.");
                                        cloud_failed = true;
                                    }
                                }
                            }

                            if cd.is_connected() {
                                // Send raw packet to cloud
                                if let Err(e) = cd
                                    .send_raw_packet(
                                        &reading.raw_data,
                                        reading.timestamp,
                                        cloud_seq,
                                    )
                                    .await
                                {
                                    warn!("[CLOUD] Failed to send packet: {e}");
                                } else {
                                    cloud_seq += 1;
                                    source = DecodeSource::Cloud;
                                    // Samples will arrive asynchronously via
                                    // cloud_decoder.try_recv_decoded() above
                                }
                            }
                        }
                    }

                    // ── Step 3: Output ────────────────────────────────
                    let source_label = match source {
                        DecodeSource::Local => "local",
                        DecodeSource::Cloud => "cloud",
                        DecodeSource::None => "raw",
                    };

                    if let Some(ref samps) = samples {
                        // We have decoded samples (local)
                        if eeg_count <= 10 || eeg_count % 100 == 0 {
                            let first = samps.first().copied().unwrap_or(f64::NAN);
                            let rms = (samps.iter().map(|v| v * v).sum::<f64>()
                                / samps.len() as f64)
                                .sqrt();
                            println!(
                                "[EEG] idx={:3}  ts={:.0} ms  {} samples  \
                                 first={:+8.3} µV  rms={:.1} µV  src={source_label}  (pkt #{})",
                                reading.index,
                                reading.timestamp,
                                samps.len(),
                                first,
                                rms,
                                eeg_count,
                            );
                        }
                        if let Some(ref mut w) = csv_writer {
                            for (si, &uv) in samps.iter().enumerate() {
                                writeln!(
                                    w,
                                    "{:.3},{},{},{:.6},{source_label}",
                                    reading.timestamp, reading.index, si, uv
                                )
                                .ok();
                            }
                        }
                    } else {
                        // No local samples — either sent to cloud or raw only
                        if eeg_count <= 10 || eeg_count % 100 == 0 {
                            println!(
                                "[EEG] idx={:3}  ts={:.0} ms  len={} bytes  \
                                 src={source_label}  (pkt #{})",
                                reading.index,
                                reading.timestamp,
                                reading.raw_data.len(),
                                eeg_count,
                            );
                        }
                        if let Some(ref mut w) = csv_writer {
                            if source == DecodeSource::Cloud {
                                // Cloud samples arrive async; raw logged here as placeholder
                                use base64::Engine;
                                let b64 = base64::engine::general_purpose::STANDARD
                                    .encode(&reading.raw_data);
                                writeln!(
                                    w,
                                    "{:.3},{},{},{source_label}",
                                    reading.timestamp, reading.index, b64
                                )
                                .ok();
                            } else {
                                use base64::Engine;
                                let b64 = base64::engine::general_purpose::STANDARD
                                    .encode(&reading.raw_data);
                                writeln!(
                                    w,
                                    "{:.3},{},{},{source_label}",
                                    reading.timestamp, reading.index, b64
                                )
                                .ok();
                            }
                        }
                    }
                } else {
                    // No decoding at all — raw mode
                    if eeg_count <= 10 || eeg_count % 100 == 0 {
                        println!(
                            "[EEG] idx={:3}  ts={:.0} ms  len={} bytes  (pkt #{})",
                            reading.index,
                            reading.timestamp,
                            reading.raw_data.len(),
                            eeg_count,
                        );
                    }
                    if let Some(ref mut w) = csv_writer {
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD
                            .encode(&reading.raw_data);
                        writeln!(w, "{:.3},{},{}", reading.timestamp, reading.index, b64).ok();
                    }
                }
            }
            GuardianEvent::Impedance(reading) => {
                println!(
                    "[IMPEDANCE] {:.2} kΩ  ({} Ω)  ts={:.0} ms",
                    reading.impedance_kohms, reading.impedance_ohms, reading.timestamp
                );
                if let Some(ref mut w) = csv_writer {
                    writeln!(
                        w,
                        "{:.3},{},{}",
                        reading.timestamp, reading.impedance_ohms, reading.impedance_kohms
                    )
                    .ok();
                }
            }
            GuardianEvent::Accelerometer(r) => {
                if eeg_count <= 5 || eeg_count % 200 == 0 {
                    println!(
                        "[ACCEL] idx={:3}  x={:+.5}g  y={:+.5}g  z={:+.5}g",
                        r.index, r.sample.x, r.sample.y, r.sample.z,
                    );
                }
            }
            GuardianEvent::Gyroscope(r) => {
                if eeg_count <= 5 || eeg_count % 200 == 0 {
                    println!(
                        "[GYRO]  idx={:3}  x={:+.3}°/s  y={:+.3}°/s  z={:+.3}°/s",
                        r.index, r.sample.x, r.sample.y, r.sample.z,
                    );
                }
            }
            GuardianEvent::Battery(b) => {
                println!("[BATTERY] {}%", b.level);
            }
            GuardianEvent::DeviceInfo(info) => {
                println!(
                    "[DEVICE] MAC={} FW={} HW={}",
                    info.mac_address, info.firmware_version, info.hardware_version
                );
            }
        }
    }

    // ── Cleanup ───────────────────────────────────────────────────────────
    if let Some(ref mut cd) = cloud_decoder {
        if cd.is_connected() {
            info!("Ending cloud recording session…");
            cd.disconnect().await.ok();
        }
    }

    if let Some(ref mut w) = csv_writer {
        w.flush()?;
        if let Some(ref path) = csv_path {
            info!("CSV data written to: {}", path.display());
        }
    }

    #[cfg(feature = "local-decode")]
    if local_fail_count > 0 {
        info!(
            "Local decode failures: {local_fail_count} / {eeg_count} packets ({:.1}%)",
            100.0 * local_fail_count as f64 / eeg_count.max(1) as f64
        );
    }

    info!("Event loop finished – exiting.");
    Ok(())
}
