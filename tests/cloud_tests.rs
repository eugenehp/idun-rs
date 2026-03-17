use idun::cloud::CloudDecoder;

#[test]
fn cloud_decoder_new_defaults() {
    let dec = CloudDecoder::new("tok".into(), "AA-BB".into());
    assert!(!dec.is_connected());
    assert!(dec.recording_id().is_none());
}

#[test]
fn cloud_decoder_try_recv_when_not_connected() {
    let mut dec = CloudDecoder::new("tok".into(), "AA-BB".into());
    // Should return Ok(None) — no channel yet
    assert!(dec.try_recv_decoded().unwrap().is_none());
}

#[test]
fn cloud_decoder_from_env_missing() {
    // Ensure the env var is unset for this test
    std::env::remove_var("IDUN_API_TOKEN");
    let result = CloudDecoder::from_env("AA-BB".into());
    assert!(result.is_err());
    let msg = result.err().expect("should be Err").to_string();
    assert!(msg.contains("IDUN_API_TOKEN"), "error should mention env var: {msg}");
}

#[test]
fn cloud_decoder_from_env_present() {
    std::env::set_var("IDUN_API_TOKEN", "test-token-123");
    let dec = CloudDecoder::from_env("AA-BB".into()).unwrap();
    assert!(!dec.is_connected());
    std::env::remove_var("IDUN_API_TOKEN");
}

#[test]
fn cloud_decoded_eeg_clone() {
    use idun::cloud::CloudDecodedEeg;
    let d = CloudDecodedEeg {
        action: "liveStreamInsights".into(),
        data: serde_json::json!({"samples": [1, 2, 3]}),
        sequence: Some(42),
    };
    let d2 = d.clone();
    assert_eq!(d2.action, "liveStreamInsights");
    assert_eq!(d2.sequence, Some(42));
}
