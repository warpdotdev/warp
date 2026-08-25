//! Tests for MCP error classification.

use std::any::TypeId;

use rmcp::transport::DynamicTransportError;
use rmcp::transport::streamable_http_client::{AuthRequiredError, StreamableHttpError};

use super::*;
use crate::runtime::{McpSpawnError, should_delete_credentials};

const EXPIRED_CHALLENGE: &str =
    r#"Bearer error="invalid_token", error_description="proxy_token_expired""#;
const STALE_CHALLENGE: &str =
    r#"Bearer error="invalid_token", error_description="proxy_token_stale""#;

fn transport_send_error(error: Box<dyn std::error::Error + Send + Sync>) -> rmcp::ServiceError {
    rmcp::ServiceError::TransportSend(DynamicTransportError::from_parts(
        "test-transport",
        TypeId::of::<()>(),
        error,
    ))
}

#[test]
fn parses_proxy_challenges() {
    assert_eq!(
        parse_www_authenticate_reason(EXPIRED_CHALLENGE),
        Some(ProxyAuthReason::ProxyTokenExpired)
    );
    assert_eq!(
        parse_www_authenticate_reason(STALE_CHALLENGE),
        Some(ProxyAuthReason::ProxyTokenStale)
    );
    // Foreign challenges (no proxy reason) and non-token errors don't match.
    assert_eq!(
        parse_www_authenticate_reason(r#"Bearer realm="downstream""#),
        None
    );
    assert_eq!(
        parse_www_authenticate_reason(
            r#"Bearer error="insufficient_scope", error_description="proxy_token_expired""#
        ),
        None
    );
}

#[test]
fn closed_and_timed_out_transports_are_transient() {
    assert_eq!(
        classify_service_error(&rmcp::ServiceError::TransportClosed),
        McpErrorClass::Transient
    );
    assert_eq!(
        classify_service_error(&rmcp::ServiceError::Timeout {
            timeout: std::time::Duration::from_secs(1)
        }),
        McpErrorClass::Transient
    );
}

#[test]
fn mcp_application_errors_are_fatal() {
    let error = rmcp::ServiceError::McpError(rmcp::model::ErrorData {
        code: rmcp::model::ErrorCode::INTERNAL_ERROR,
        message: "boom".into(),
        data: None,
    });
    assert_eq!(classify_service_error(&error), McpErrorClass::Fatal);
}

#[test]
fn streamable_http_auth_required_uses_the_challenge() {
    let expired = transport_send_error(Box::new(
        StreamableHttpError::<reqwest::Error>::AuthRequired(AuthRequiredError::new(
            EXPIRED_CHALLENGE.to_string(),
        )),
    ));
    assert_eq!(
        classify_service_error(&expired),
        McpErrorClass::AuthExpiredRecoverable(ProxyAuthReason::ProxyTokenExpired)
    );

    let foreign = transport_send_error(Box::new(
        StreamableHttpError::<reqwest::Error>::AuthRequired(AuthRequiredError::new(
            r#"Bearer realm="downstream""#.to_string(),
        )),
    ));
    assert_eq!(
        classify_service_error(&foreign),
        McpErrorClass::AuthRequiresUser
    );
}

#[test]
fn streamable_http_server_responses_classify_by_status_and_body() {
    let classify = |message: &str| {
        classify_service_error(&transport_send_error(Box::new(StreamableHttpError::<
            reqwest::Error,
        >::UnexpectedServerResponse(
            message.to_string().into(),
        ))))
    };

    // A proxy code in the body wins regardless of challenge stripping.
    assert_eq!(
        classify(r#"HTTP 401 Unauthorized: {"error":"expired","code":"proxy_token_expired"}"#),
        McpErrorClass::AuthExpiredRecoverable(ProxyAuthReason::ProxyTokenExpired)
    );
    // A bare 401 through the proxy means the downstream rejected auth.
    assert_eq!(
        classify("HTTP 401 Unauthorized: nope"),
        McpErrorClass::AuthRequiresUser
    );
    // The proxy's dead-grant denial code is a user-fixable auth problem.
    assert_eq!(
        classify(r#"HTTP 502 Bad Gateway: {"error":"...","code":"downstream_auth_failed"}"#),
        McpErrorClass::AuthRequiresUser
    );
    assert_eq!(
        classify("HTTP 503 Service Unavailable: try later"),
        McpErrorClass::Transient
    );
    assert_eq!(classify("HTTP 400 Bad Request: nope"), McpErrorClass::Fatal);
}

#[test]
fn nested_oauth_sse_errors_are_unwrapped() {
    // The OAuth SSE path wraps the plain transport error inside `Client`.
    let nested: SseTransportError<SseTransportError<reqwest::Error>> =
        SseTransportError::Client(SseTransportError::HttpStatus {
            status: http::StatusCode::UNAUTHORIZED,
            body: String::new(),
            www_authenticate: Some(STALE_CHALLENGE.to_string()),
        });
    assert_eq!(
        classify_service_error(&transport_send_error(Box::new(nested))),
        McpErrorClass::AuthExpiredRecoverable(ProxyAuthReason::ProxyTokenStale)
    );
}

#[test]
fn sse_http_statuses_classify_like_http_responses() {
    let classify = |error: SseTransportError<reqwest::Error>| {
        classify_service_error(&transport_send_error(Box::new(error)))
    };

    assert_eq!(
        classify(SseTransportError::HttpStatus {
            status: http::StatusCode::FORBIDDEN,
            body: String::new(),
            www_authenticate: None,
        }),
        McpErrorClass::AuthRequiresUser
    );
    assert_eq!(
        classify(SseTransportError::HttpStatus {
            status: http::StatusCode::BAD_GATEWAY,
            body: r#"{"error":"...","code":"downstream_auth_failed"}"#.to_string(),
            www_authenticate: None,
        }),
        McpErrorClass::AuthRequiresUser
    );
    assert_eq!(
        classify(SseTransportError::UnexpectedEndOfStream),
        McpErrorClass::Transient
    );
}

#[test]
fn unknown_transport_errors_are_transient() {
    // E.g. a stdio child whose pipe broke.
    let error = transport_send_error(Box::new(std::io::Error::other("broken pipe")));
    assert_eq!(classify_service_error(&error), McpErrorClass::Transient);
}

#[test]
fn credentials_are_deleted_only_on_unrecoverable_auth_rejections() {
    assert!(should_delete_credentials(&McpSpawnError::AuthRequired {
        www_authenticate: None,
        reason: None,
        message: "rejected".to_string(),
    }));
    // Re-mintable proxy expiry heals without re-auth; keep credentials.
    assert!(!should_delete_credentials(&McpSpawnError::AuthRequired {
        www_authenticate: Some(EXPIRED_CHALLENGE.to_string()),
        reason: Some(ProxyAuthReason::ProxyTokenExpired),
        message: "expired".to_string(),
    }));
    // Transient failures must never log the user out.
    assert!(!should_delete_credentials(&McpSpawnError::Other(
        rmcp::RmcpError::transport_creation::<
            rmcp::transport::StreamableHttpClientTransport<reqwest::Client>,
        >("dns failure".to_string()),
    )));
}
