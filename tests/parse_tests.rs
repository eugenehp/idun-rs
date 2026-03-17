use idun::parse::*;

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

// ── impedance edge cases ──────────────────────────────────────────────────

#[test]
fn parse_impedance_3byte_le() {
    // 3 bytes: [0x40, 0x42, 0x0F] → 0x000F4240 = 1_000_000
    assert_eq!(parse_impedance(&[0x40, 0x42, 0x0F]), Some(0x000F4240));
}

#[test]
fn parse_impedance_large_4byte() {
    // Max u32
    assert_eq!(
        parse_impedance(&[0xFF, 0xFF, 0xFF, 0xFF]),
        Some(u32::MAX)
    );
}

#[test]
fn parse_impedance_extra_bytes_ignored() {
    // More than 4 bytes — only first 4 used
    assert_eq!(
        parse_impedance(&[0x01, 0x00, 0x00, 0x00, 0xFF, 0xFF]),
        Some(1)
    );
}

#[test]
fn parse_eeg_packet_max_index() {
    let data = vec![0x00, 0xFF, 0xAA];
    let header = parse_eeg_packet(&data).unwrap();
    assert_eq!(header.index, 255);
}

#[test]
fn parse_eeg_packet_large_payload() {
    // Realistic 32-byte payload
    let mut data = vec![0xBB, 0x10];
    data.extend_from_slice(&[0x80; 30]);
    let header = parse_eeg_packet(&data).unwrap();
    assert_eq!(header.tag, 0xBB);
    assert_eq!(header.index, 16);
    assert_eq!(header.payload.len(), 30);
}

// ── local-decode feature-gated tests ──────────────────────────────────────

#[cfg(feature = "local-decode")]
mod local_decode {
    use idun::parse::*;

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
    fn try_decode_eeg_12bit_max_values() {
        // All 0xFF → s0 = (0xFF<<4)|(0xFF>>4) = 0xFFF = 4095
        //            s1 = ((0xFF&0x0F)<<8)|0xFF = 0xFFF = 4095
        // voltage = 0.48828125 * (4095 - 2048) = 0.48828125 * 2047 ≈ 999.51
        let samples = try_decode_eeg_12bit(&[0xFF, 0xFF, 0xFF]);
        assert_eq!(samples.len(), 2);
        assert!((samples[0] - 999.511).abs() < 0.01);
        assert!((samples[1] - 999.511).abs() < 0.01);
    }

    #[test]
    fn try_decode_eeg_12bit_min_values() {
        // All 0x00 → s0=0, s1=0 → voltage = 0.488 * (0 - 2048) = -1000.0
        let samples = try_decode_eeg_12bit(&[0x00, 0x00, 0x00]);
        assert_eq!(samples.len(), 2);
        assert!((samples[0] - (-1000.0)).abs() < 0.001);
        assert!((samples[1] - (-1000.0)).abs() < 0.001);
    }

    #[test]
    fn try_decode_imu_negative_values() {
        // ax = -1000 (LE: i16 -1000 = 0xFC18 → [0x18, 0xFC])
        let mut data = [0u8; 12];
        let neg: i16 = -1000;
        let bytes = neg.to_le_bytes();
        data[0] = bytes[0]; data[1] = bytes[1]; // ax = -1000
        data[6] = bytes[0]; data[7] = bytes[1]; // gx = -1000
        let (accel, gyro) = try_decode_imu_i16le(&data).unwrap();
        assert!(accel.x < 0.0);
        assert!((accel.x - (-1000.0 * 0.0000610352)).abs() < 0.001);
        assert!(gyro.x < 0.0);
        assert!((gyro.x - (-1000.0 * 0.0074768)).abs() < 0.01);
    }

    #[test]
    fn try_decode_imu_exact_12_bytes() {
        // Exactly 12 bytes should work
        let data = [0u8; 12];
        assert!(try_decode_imu_i16le(&data).is_some());
    }

    #[test]
    fn try_decode_imu_extra_bytes() {
        // More than 12 bytes — only first 12 used
        let data = [0u8; 20];
        assert!(try_decode_imu_i16le(&data).is_some());
    }

    #[test]
    fn compute_rms_single_value() {
        assert!((compute_rms(&[7.0]) - 7.0).abs() < 0.001);
    }

    #[test]
    fn compute_rms_negative_values() {
        // RMS of [-3, -3, -3] = 3
        assert!((compute_rms(&[-3.0, -3.0, -3.0]) - 3.0).abs() < 0.001);
    }

    #[test]
    fn parse_notification_unknown_short_payload() {
        // 2-byte header + 5-byte payload (too short for EEG or IMU)
        let data = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
        let results = parse_notification(&data);
        assert!(!results.is_empty());
        // Should be Unknown since payload is too short for decode
        match &results[0] {
            ParsedPacket::Unknown { index, tag, .. } => {
                assert_eq!(*tag, 0x01);
                assert_eq!(*index, 0x02);
            }
            _ => panic!("Expected Unknown variant for short payload"),
        }
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
