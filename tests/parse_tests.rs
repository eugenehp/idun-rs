use idun_rs::parse::*;

#[test]
fn parse_eeg_packet_too_short() {
    assert!(parse_eeg_packet(&[]).is_none());
    assert!(parse_eeg_packet(&[0x01]).is_none());
}

#[test]
fn parse_eeg_packet_header() {
    let data = vec![0xAA, 42, 1, 2, 3, 4, 5];
    let header = parse_eeg_packet(&data).unwrap();
    assert_eq!(header.tag, 0xAA);
    assert_eq!(header.index, 42);
    assert_eq!(header.payload, vec![1, 2, 3, 4, 5]);
}

#[test]
fn parse_eeg_packet_minimal() {
    let data = vec![0x00, 0xFF];
    let header = parse_eeg_packet(&data).unwrap();
    assert_eq!(header.tag, 0x00);
    assert_eq!(header.index, 255);
    assert!(header.payload.is_empty());
}

#[test]
fn parse_impedance_empty() {
    assert!(parse_impedance(&[]).is_none());
}

#[test]
fn parse_impedance_1byte() {
    assert_eq!(parse_impedance(&[200]), Some(200));
}

#[test]
fn parse_impedance_2byte_le() {
    // 0x1234 = 4660 in LE: [0x34, 0x12]
    assert_eq!(parse_impedance(&[0x34, 0x12]), Some(0x1234));
}

#[test]
fn parse_impedance_4byte_le() {
    // 5000 ohms = 0x00001388 in LE: [0x88, 0x13, 0x00, 0x00]
    assert_eq!(parse_impedance(&[0x88, 0x13, 0x00, 0x00]), Some(5000));
}

// ── local-decode feature-gated tests ──────────────────────────────────────

#[cfg(feature = "local-decode")]
mod local_decode {
    use idun_rs::parse::*;

    #[test]
    fn try_decode_eeg_12bit_empty() {
        let samples = try_decode_eeg_12bit(&[]);
        assert!(samples.is_empty());
    }

    #[test]
    fn try_decode_eeg_12bit_one_group() {
        // 3 bytes → 2 samples
        // bytes: [0x80, 0x00, 0x00]
        // sample0 = (0x80 << 4) | (0x00 >> 4) = 0x800 = 2048 → (2048-2048)*0.488 = 0.0
        // sample1 = ((0x00 & 0x0F) << 8) | 0x00 = 0 → (0-2048)*0.488 = -1000.0
        let samples = try_decode_eeg_12bit(&[0x80, 0x00, 0x00]);
        assert_eq!(samples.len(), 2);
        assert!((samples[0] - 0.0).abs() < 0.001);
        assert!((samples[1] - (-1000.0)).abs() < 0.001);
    }

    #[test]
    fn try_decode_eeg_12bit_mid_scale() {
        // Both samples at mid-scale (2048 = 0x800)
        // s0 = (data[0] << 4) | (data[1] >> 4) = (0x80 << 4) | (0x08 >> 4) = 0x800
        // s1 = ((data[1] & 0x0F) << 8) | data[2] = (0x08 & 0x0F) << 8 | 0x00 = 0x800
        let samples = try_decode_eeg_12bit(&[0x80, 0x08, 0x00]);
        assert_eq!(samples.len(), 2);
        assert!((samples[0] - 0.0).abs() < 0.001);
        assert!((samples[1] - 0.0).abs() < 0.001);
    }

    #[test]
    fn try_decode_eeg_12bit_multiple_groups() {
        // 6 bytes → 4 samples
        let payload = [0x80, 0x08, 0x00, 0x80, 0x08, 0x00];
        let samples = try_decode_eeg_12bit(&payload);
        assert_eq!(samples.len(), 4);
        for s in &samples {
            assert!((s - 0.0).abs() < 0.001);
        }
    }

    #[test]
    fn try_decode_eeg_12bit_partial_group_ignored() {
        // 4 bytes → only first group (3 bytes) decoded = 2 samples
        // remaining 1 byte ignored
        let samples = try_decode_eeg_12bit(&[0x80, 0x08, 0x00, 0xFF]);
        assert_eq!(samples.len(), 2);
    }

    #[test]
    fn compute_rms_empty() {
        assert_eq!(compute_rms(&[]), 0.0);
    }

    #[test]
    fn compute_rms_dc() {
        let samples = vec![10.0, 10.0, 10.0, 10.0];
        assert!((compute_rms(&samples) - 10.0).abs() < 0.001);
    }

    #[test]
    fn compute_rms_symmetric() {
        let samples = vec![-5.0, 5.0, -5.0, 5.0];
        assert!((compute_rms(&samples) - 5.0).abs() < 0.001);
    }

    #[test]
    fn try_decode_imu_too_short() {
        assert!(try_decode_imu_i16le(&[0; 11]).is_none());
    }

    #[test]
    fn try_decode_imu_zeros() {
        let data = [0u8; 12];
        let (accel, gyro) = try_decode_imu_i16le(&data).unwrap();
        assert_eq!(accel.x, 0.0);
        assert_eq!(accel.y, 0.0);
        assert_eq!(accel.z, 0.0);
        assert_eq!(gyro.x, 0.0);
        assert_eq!(gyro.y, 0.0);
        assert_eq!(gyro.z, 0.0);
    }

    #[test]
    fn try_decode_imu_nonzero() {
        // ax=1000 (LE: [0xE8, 0x03]), ay=0, az=0, gx=1000, gy=0, gz=0
        let mut data = [0u8; 12];
        data[0] = 0xE8; data[1] = 0x03; // ax = 1000
        data[6] = 0xE8; data[7] = 0x03; // gx = 1000
        let (accel, gyro) = try_decode_imu_i16le(&data).unwrap();
        assert!((accel.x - 1000.0 * 0.0000610352).abs() < 0.001);
        assert!((gyro.x - 1000.0 * 0.0074768).abs() < 0.01);
    }

    #[test]
    fn parse_notification_short() {
        assert!(parse_notification(&[]).is_empty());
        assert!(parse_notification(&[0x01]).is_empty());
    }

    #[test]
    fn parse_notification_produces_results() {
        // 2-byte header + 30-byte payload (enough for EEG + IMU)
        let mut data = vec![0xAA, 0x01];
        data.extend_from_slice(&[0x80, 0x08, 0x00].repeat(10)); // 30 bytes of EEG-like data
        let results = parse_notification(&data);
        assert!(!results.is_empty());
    }
}
