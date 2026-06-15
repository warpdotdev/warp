use anyhow::anyhow;

use super::{AIApiError, DeserializationError};

/// Transient server-side statuses are resume-eligible; client errors are not.
#[test]
fn error_status_recoverability_follows_status_class() {
    assert!(
        AIApiError::ErrorStatus(http::StatusCode::INTERNAL_SERVER_ERROR, "boom".into())
            .is_recoverable()
    );
    assert!(AIApiError::ErrorStatus(http::StatusCode::BAD_GATEWAY, "boom".into()).is_recoverable());
    assert!(
        AIApiError::ErrorStatus(http::StatusCode::REQUEST_TIMEOUT, "slow".into()).is_recoverable()
    );
    assert!(!AIApiError::ErrorStatus(http::StatusCode::NOT_FOUND, "nope".into()).is_recoverable());
    assert!(
        !AIApiError::ErrorStatus(http::StatusCode::UNPROCESSABLE_ENTITY, "bad".into())
            .is_recoverable()
    );
}

/// Application-level and misc failures are all recoverable — a fresh request may
/// still succeed.
#[test]
fn application_level_and_misc_failures_are_recoverable() {
    assert!(AIApiError::QuotaLimit {
        user_display_message: None,
    }
    .is_recoverable());
    assert!(AIApiError::ServerOverloaded.is_recoverable());
    assert!(AIApiError::NoContextFound.is_recoverable());
    assert!(AIApiError::Other(anyhow!("misc")).is_recoverable());
    assert!(AIApiError::Stream {
        stream_type: "test",
        source: anyhow!("protocol error"),
    }
    .is_recoverable());

    let json_err = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
    assert!(AIApiError::Deserialization(DeserializationError::Json(json_err)).is_recoverable());
}

/// An unexpected EOF (clean transport cut before the finished event) is recoverable.
#[test]
fn unexpected_eof_is_recoverable() {
    assert!(AIApiError::UnexpectedEof.is_recoverable());
}
