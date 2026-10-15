use idun::protocol::*;
use uuid::Uuid;

#[test]
fn device_name_prefix_matches_guardian() {
    assert!("IGEB".starts_with(DEVICE_NAME_PREFIX));
    assert!("IGE-ABC123".starts_with(DEVICE_NAME_PREFIX));
    assert!(!"Muse-1234".starts_with(DEVICE_NAME_PREFIX));
}

#[test]
fn sampling_constants_consistent() {
    // 20 samples at 250 Hz = 80 ms per packet
    let packet_duration_ms = 1000.0 * (EEG_SAMPLES_PER_PACKET as f64) / EEG_SAMPLE_RATE;
    assert!((packet_duration_ms - 80.0).abs() < 0.001);
}

#[test]
fn max_packet_index_is_256() {
    assert_eq!(MAX_PACKET_INDEX, 256);
}

#[test]
fn command_bytes_are_ascii() {
    assert_eq!(CMD_START_MEASUREMENT, b"M");
    assert_eq!(CMD_STOP_MEASUREMENT, b"S");
    assert_eq!(CMD_START_IMPEDANCE, b"Z");
    assert_eq!(CMD_STOP_IMPEDANCE, b"X");
}

#[test]
fn config_bytes_are_ascii() {
    assert_eq!(CFG_LED_ON, b"d1");
    assert_eq!(CFG_LED_OFF, b"d0");
    assert_eq!(CFG_NOTCH_50HZ, b"n0");
    assert_eq!(CFG_NOTCH_60HZ, b"n1");
}

#[test]
fn uuids_are_distinct() {
    let uuids = [
        EEG_IMU_CHARACTERISTIC,
        IMPEDANCE_CHARACTERISTIC,
        CONFIG_CHARACTERISTIC,
        COMMAND_CHARACTERISTIC,
        BATTERY_CHARACTERISTIC,
        MAC_ID_CHARACTERISTIC,
        FIRMWARE_VERSION_CHARACTERISTIC,
        HARDWARE_VERSION_CHARACTERISTIC,
    ];
    for i in 0..uuids.len() {
        for j in (i + 1)..uuids.len() {
            assert_ne!(uuids[i], uuids[j], "UUID collision at index {i} and {j}");
        }
    }
}

#[test]
fn device_info_service_is_standard_sig() {
    // 0x180A is the standard Device Information Service UUID
    let expected = Uuid::from_u128(0x0000180a_0000_1000_8000_00805f9b34fb);
    assert_eq!(DEVICE_INFO_SERVICE, expected);
}

#[test]
fn eeg_channel_name_is_eeg() {
    assert_eq!(EEG_CHANNEL_NAME, "EEG");
}

#[test]
fn battery_characteristic_is_standard_sig() {
    // 0x2A19 is the standard Battery Level UUID
    let expected = Uuid::from_u128(0x00002a19_0000_1000_8000_00805f9b34fb);
    assert_eq!(BATTERY_CHARACTERISTIC, expected);
}

#[test]
fn vendor_uuids_share_base() {
    // All vendor UUIDs should start with beffd56c-c915-48f5-930d
    let vendor_uuids = [
        EEG_IMU_CHARACTERISTIC,
        IMPEDANCE_CHARACTERISTIC,
        CONFIG_CHARACTERISTIC,
        COMMAND_CHARACTERISTIC,
    ];
    for uuid in &vendor_uuids {
        let s = uuid.to_string();
        assert!(
            s.starts_with("beffd56c-c915-48f5-930d"),
            "UUID {s} doesn't share vendor base"
        );
    }
}

#[test]
fn guardian_service_shares_vendor_base() {
    assert_eq!(
        GUARDIAN_SERVICE.to_string(),
        "beffd56c-c915-48f5-930d-4c1feee0fcc3"
    );
}

#[test]
fn device_id_characteristic_uuid() {
    assert_eq!(
        DEVICE_ID_CHARACTERISTIC.to_string(),
        "beffd56c-c915-48f5-930d-4c1feee0fccb"
    );
}

#[test]
fn battery_service_is_standard_sig() {
    let expected = Uuid::parse_str("0000180f-0000-1000-8000-00805f9b34fb").unwrap();
    assert_eq!(BATTERY_SERVICE, expected);
}
