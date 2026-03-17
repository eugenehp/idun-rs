use idun_rs::types::*;

#[test]
fn eeg_reading_debug() {
    let r = EegReading {
        index: 42,
        timestamp: 1234567890.0,
        raw_data: vec![0xAA, 42, 1, 2, 3],
        samples: None,
        decode_source: DecodeSource::None,
    };
    let dbg = format!("{:?}", r);
    assert!(dbg.contains("42"));
    assert!(dbg.contains("1234567890"));
}

#[test]
fn impedance_reading_kohms() {
    let r = ImpedanceReading {
        impedance_ohms: 5000,
        impedance_kohms: 5.0,
        timestamp: 0.0,
    };
    assert_eq!(r.impedance_kohms, r.impedance_ohms as f64 / 1000.0);
}

#[test]
fn battery_reading_range() {
    let b = BatteryReading { level: 100 };
    assert!(b.level <= 100);
}

#[test]
fn device_info_fields() {
    let d = DeviceInfo {
        mac_address: "AA-BB-CC-DD-EE-FF".into(),
        firmware_version: "1.2.3".into(),
        hardware_version: "3.0a".into(),
    };
    assert!(d.mac_address.contains('-'));
    assert!(!d.firmware_version.is_empty());
    assert!(!d.hardware_version.is_empty());
}

#[test]
fn xyz_sample_default() {
    let s = XyzSample::default();
    assert_eq!(s.x, 0.0);
    assert_eq!(s.y, 0.0);
    assert_eq!(s.z, 0.0);
}

#[test]
fn accelerometer_reading() {
    let r = AccelerometerReading {
        index: 10,
        timestamp: 1000.0,
        sample: XyzSample { x: 0.01, y: -0.02, z: -0.98 },
    };
    assert!((r.sample.z + 0.98).abs() < 0.001);
}

#[test]
fn gyroscope_reading() {
    let r = GyroscopeReading {
        index: 10,
        timestamp: 1000.0,
        sample: XyzSample { x: 1.5, y: -0.3, z: 0.0 },
    };
    assert_eq!(r.sample.x, 1.5);
}

#[test]
fn guardian_event_all_variants() {
    let events: Vec<GuardianEvent> = vec![
        GuardianEvent::Connected("IGEB".into()),
        GuardianEvent::Disconnected,
        GuardianEvent::Eeg(EegReading {
            index: 0,
            timestamp: 0.0,
            raw_data: vec![],
            samples: None,
            decode_source: DecodeSource::None,
        }),
        GuardianEvent::Accelerometer(AccelerometerReading {
            index: 0,
            timestamp: 0.0,
            sample: XyzSample::default(),
        }),
        GuardianEvent::Gyroscope(GyroscopeReading {
            index: 0,
            timestamp: 0.0,
            sample: XyzSample::default(),
        }),
        GuardianEvent::Impedance(ImpedanceReading {
            impedance_ohms: 1000,
            impedance_kohms: 1.0,
            timestamp: 0.0,
        }),
        GuardianEvent::Battery(BatteryReading { level: 50 }),
        GuardianEvent::DeviceInfo(DeviceInfo {
            mac_address: String::new(),
            firmware_version: String::new(),
            hardware_version: String::new(),
        }),
    ];
    assert_eq!(events.len(), 8);
}

#[test]
fn guardian_event_clone() {
    let ev = GuardianEvent::Battery(BatteryReading { level: 75 });
    let ev2 = ev.clone();
    if let GuardianEvent::Battery(b) = ev2 {
        assert_eq!(b.level, 75);
    } else {
        panic!("Clone changed variant");
    }
}

#[test]
fn decode_source_equality() {
    assert_eq!(DecodeSource::None, DecodeSource::None);
    assert_ne!(DecodeSource::Local, DecodeSource::Cloud);
}
