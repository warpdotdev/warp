use super::{IapConfig, WarpServerConfig};

#[test]
fn server_root_url_override_disables_iap() {
    let mut config = WarpServerConfig::production();
    config.iap_config = Some(IapConfig {
        audiences: "staging-audience".into(),
        service_account_email: "iap@example.com".into(),
    });

    config
        .override_server_root_url("http://localhost:8080")
        .unwrap();

    assert_eq!(config.server_root_url, "http://localhost:8080");
    assert!(config.iap_config.is_none());
}
