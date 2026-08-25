//! Tests for the reqwest `SseClient` impl's HTTP error capture.

use axum::Router;
use axum::routing::{get, post};

use super::sse_client::{SseClient as _, SseTransportError};

const EXPIRED_CHALLENGE: &str =
    r#"Bearer error="invalid_token", error_description="proxy_token_expired""#;
const ERROR_BODY: &str = r#"{"error":"proxy session expired","code":"proxy_token_expired"}"#;

async fn serve(router: Router) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    addr
}

async fn unauthorized() -> impl axum::response::IntoResponse {
    (
        axum::http::StatusCode::UNAUTHORIZED,
        [(axum::http::header::WWW_AUTHENTICATE, EXPIRED_CHALLENGE)],
        ERROR_BODY,
    )
}

fn test_message() -> rmcp::model::ClientJsonRpcMessage {
    let request = rmcp::model::InitializeRequest::new(rmcp::model::ClientInfo::new(
        Default::default(),
        rmcp::model::Implementation::new("test".to_string(), "0.0.0".to_string()),
    ));
    rmcp::model::ClientJsonRpcMessage::request(
        rmcp::model::ClientRequest::InitializeRequest(request),
        rmcp::model::RequestId::Number(0),
    )
}

#[tokio::test]
async fn post_message_captures_status_body_and_challenge() {
    let addr = serve(Router::new().route("/mcp", post(unauthorized))).await;
    let uri: http::Uri = format!("http://{addr}/mcp").parse().expect("uri");

    let error = reqwest::Client::default()
        .post_message(uri, test_message(), None)
        .await
        .expect_err("401 should error");

    match error {
        SseTransportError::HttpStatus {
            status,
            body,
            www_authenticate,
        } => {
            assert_eq!(status, http::StatusCode::UNAUTHORIZED);
            assert_eq!(body, ERROR_BODY);
            assert_eq!(www_authenticate.as_deref(), Some(EXPIRED_CHALLENGE));
        }
        other => panic!("expected HttpStatus, got: {other:?}"),
    }
}

#[tokio::test]
async fn get_stream_captures_status_body_and_challenge() {
    let addr = serve(Router::new().route("/sse", get(unauthorized))).await;
    let uri: http::Uri = format!("http://{addr}/sse").parse().expect("uri");

    let error = match reqwest::Client::default().get_stream(uri, None, None).await {
        Ok(_) => panic!("401 should error"),
        Err(error) => error,
    };

    match error {
        SseTransportError::HttpStatus {
            status,
            body,
            www_authenticate,
        } => {
            assert_eq!(status, http::StatusCode::UNAUTHORIZED);
            assert_eq!(body, ERROR_BODY);
            assert_eq!(www_authenticate.as_deref(), Some(EXPIRED_CHALLENGE));
        }
        other => panic!("expected HttpStatus, got: {other:?}"),
    }
}

#[tokio::test]
async fn error_bodies_are_bounded() {
    let addr = serve(Router::new().route(
        "/mcp",
        post(|| async {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "x".repeat(1024 * 1024),
            )
        }),
    ))
    .await;
    let uri: http::Uri = format!("http://{addr}/mcp").parse().expect("uri");

    let error = reqwest::Client::default()
        .post_message(uri, test_message(), None)
        .await
        .expect_err("500 should error");

    match error {
        SseTransportError::HttpStatus { body, .. } => {
            assert_eq!(body.len(), super::reqwest_impl::MAX_ERROR_BODY_BYTES);
        }
        other => panic!("expected HttpStatus, got: {other:?}"),
    }
}
