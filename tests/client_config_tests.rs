use idun::guardian_client::GuardianClientConfig;
use idun::protocol::DEVICE_NAME_PREFIX;

#[test]
fn default_config_values() {
    let cfg = GuardianClientConfig::default();
    assert!(!cfg.mains_freq_60hz);
    assert_eq!(cfg.scan_timeout_secs, 15);
    assert_eq!(cfg.name_prefix, DEVICE_NAME_PREFIX);
}

#[test]
fn custom_config() {
    let cfg = GuardianClientConfig {
        mains_freq_60hz: true,
        scan_timeout_secs: 30,
        name_prefix: "TEST".into(),
        api_token: Some("tok-abc".into()),
    };
    assert!(cfg.mains_freq_60hz);
    assert_eq!(cfg.scan_timeout_secs, 30);
    assert_eq!(cfg.name_prefix, "TEST");
    assert_eq!(cfg.api_token.as_deref(), Some("tok-abc"));
}

#[test]
fn api_token_default_is_none() {
    let cfg = GuardianClientConfig::default();
    assert!(cfg.api_token.is_none());
}

#[test]
fn config_clone() {
    let cfg = GuardianClientConfig::default();
    let cfg2 = cfg.clone();
    assert_eq!(cfg2.scan_timeout_secs, cfg.scan_timeout_secs);
    assert_eq!(cfg2.mains_freq_60hz, cfg.mains_freq_60hz);
}
