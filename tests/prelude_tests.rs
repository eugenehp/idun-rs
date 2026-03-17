/// Verify that all expected types are accessible through the prelude.
use idun::prelude::*;

#[test]
fn prelude_exports_client_types() {
    let _cfg = GuardianClientConfig::default();
    // GuardianClient, GuardianDevice, GuardianHandle are also re-exported
    // but need BLE to construct — just verify the type names compile.
    let _: fn() -> GuardianClient = || GuardianClient::new(GuardianClientConfig::default());
}

#[test]
fn prelude_exports_event_types() {
    let _ = GuardianEvent::Disconnected;
    let _ = DecodeSource::None;
    let _ = XyzSample::default();
    let _ = BatteryReading { level: 50 };
}

#[test]
fn prelude_exports_constants() {
    assert_eq!(EEG_CHANNEL_NAME, "EEG");
    assert_eq!(EEG_SAMPLE_RATE, 250.0);
    assert_eq!(EEG_SAMPLES_PER_PACKET, 20);
}

#[test]
fn prelude_exports_parse_functions() {
    assert!(parse_eeg_packet(&[]).is_none());
    assert!(parse_impedance(&[]).is_none());
}

#[cfg(feature = "local-decode")]
#[test]
fn prelude_exports_local_decode_functions() {
    let _ = try_decode_eeg_12bit(&[]);
    let _ = try_decode_imu_i16le(&[0u8; 12]);
    let _ = compute_rms(&[1.0, 2.0]);
}
