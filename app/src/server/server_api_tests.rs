use anyhow::anyhow;

use super::{AIApiError, DeserializationError};

/// Transient server-side statuses are resume-eligible; client errors are not.
#[test]
fn error_status_transience_follows_status_class() {
    assert!(
        AIApiError::ErrorStatus(http::StatusCode::INTERNAL_SERVER_ERROR, "boom".into())
            .is_transient_failure()
    );
    assert!(
        AIApiError::ErrorStatus(http::StatusCode::BAD_GATEWAY, "boom".into())
            .is_transient_failure()
    );
    assert!(
        AIApiError::ErrorStatus(http::StatusCode::REQUEST_TIMEOUT, "slow".into())
            .is_transient_failure()
    );
    assert!(
        !AIApiError::ErrorStatus(http::StatusCode::NOT_FOUND, "nope".into()).is_transient_failure()
    );
    assert!(
        !AIApiError::ErrorStatus(http::StatusCode::UNPROCESSABLE_ENTITY, "bad".into())
            .is_transient_failure()
    );
}

/// Application-level failures must not trigger an automatic conversation resume, even
/// though the in-request retry path treats them as retryable: a resume would fail
/// identically (quota) or add load the server asked us to shed (overloaded).
#[test]
fn application_level_failures_are_not_transient() {
    let quota = AIApiError::QuotaLimit {
        user_display_message: None,
    };
    assert!(
        quota.is_retryable(),
        "precondition: quota is retryable in-request"
    );
    assert!(!quota.is_transient_failure());

    assert!(AIApiError::ServerOverloaded.is_retryable());
    assert!(!AIApiError::ServerOverloaded.is_transient_failure());

    assert!(!AIApiError::NoContextFound.is_transient_failure());
    assert!(!AIApiError::Other(anyhow!("misc")).is_transient_failure());
    assert!(!AIApiError::Stream {
        stream_type: "test",
        source: anyhow!("protocol error"),
    }
    .is_transient_failure());

    let json_err = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
    assert!(
        !AIApiError::Deserialization(DeserializationError::Json(json_err)).is_transient_failure()
    );
}

/// A truncated stream is a transport-level failure: the request can be retried and the
/// conversation is resume-eligible.
#[test]
fn stream_truncated_is_retryable_and_transient() {
    assert!(AIApiError::StreamTruncated.is_retryable());
    assert!(AIApiError::StreamTruncated.is_transient_failure());
}
