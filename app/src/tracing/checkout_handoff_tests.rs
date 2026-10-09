use chrono::{TimeDelta, Utc};

use super::*;
use crate::tracing::cloud_agent_auth::AuthContext;

fn config() -> CheckoutTracingConfig {
    serde_json::from_value(serde_json::json!({
        "endpoint": "https://collector.example.com",
        "credential": {
            "token": "handoff-test-token",
            "expires_at": Utc::now() + TimeDelta::minutes(17),
        }
    }))
    .unwrap()
}

#[test]
fn request_with_otlp_extension_is_not_consumed_as_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.otlp");
    fs::write(&requests_file, "checkout requests").unwrap();
    let guard = config().write(&requests_file).unwrap();
    assert!(
        CheckoutTracingConfig::consume(&requests_file)
            .unwrap()
            .is_some()
    );
    assert!(!guard.exists());
    assert_eq!(
        fs::read_to_string(&requests_file).unwrap(),
        "checkout requests"
    );
}

#[test]
fn handoff_preserves_credential_and_expiry_then_disappears() {
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.json");
    let config = config();
    let expected = serde_json::to_value(&config).unwrap();
    let guard = config.write(&requests_file).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&guard).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    let received = CheckoutTracingConfig::consume(&requests_file)
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_value(&received).unwrap(), expected);
    assert!(!guard.exists());
    assert!(AuthContext::from_snapshot(received.credential).is_ok());
    assert!(
        CheckoutTracingConfig::consume(&requests_file)
            .unwrap()
            .is_none()
    );
}

#[test]
fn dropping_unconsumed_handoff_cleans_up() {
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.json");
    let guard = config().write(&requests_file).unwrap();
    let path = guard.to_path_buf();
    drop(guard);
    assert!(!path.exists());
}

#[test]
fn malformed_handoff_is_removed_without_echoing_values() {
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.json");
    let guard = config().write(&requests_file).unwrap();
    fs::write(
        &guard,
        br#"{"endpoint":"secret-test-token","credential":{"expires_at":"secret-test-token"}}"#,
    )
    .unwrap();

    let error = CheckoutTracingConfig::consume(&requests_file)
        .err()
        .unwrap();
    assert!(!format!("{error:?}").contains("secret-test-token"));
    assert!(!guard.exists());
}

#[test]
fn oversized_handoff_is_removed() {
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.json");
    let guard = config().write(&requests_file).unwrap();
    fs::write(&guard, vec![b'x'; MAX_HANDOFF_BYTES as usize + 1]).unwrap();
    assert!(CheckoutTracingConfig::consume(&requests_file).is_err());
    assert!(!guard.exists());
}

#[cfg(unix)]
#[test]
fn exposed_handoff_is_rejected_and_removed() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.json");
    let guard = config().write(&requests_file).unwrap();
    fs::set_permissions(&guard, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(CheckoutTracingConfig::consume(&requests_file).is_err());
    assert!(!guard.exists());
}

#[cfg(unix)]
#[test]
fn handoff_symlink_does_not_read_or_remove_target() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.json");
    let target = directory.path().join("other-secret");
    fs::write(&target, "untouched").unwrap();
    symlink(&target, handoff_path(&requests_file)).unwrap();
    assert!(CheckoutTracingConfig::consume(&requests_file).is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
}
