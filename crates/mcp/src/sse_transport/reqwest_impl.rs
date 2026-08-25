// This file contains code copied from the rmcp crate (https://github.com/modelcontextprotocol/rust-sdk),
// originally located at `crates/rmcp/src/transport/common/reqwest/sse_client.rs`.
// Used under the terms of the Apache License, Version 2.0.
// See https://github.com/modelcontextprotocol/rust-sdk/blob/main/LICENSE for the full license text.

use std::sync::Arc;

use futures::StreamExt;
use http::Uri;
use reqwest::header::ACCEPT;
use sse_stream::SseStream;

use super::sse_client::{SseClient, SseClientConfig, SseClientTransport, SseTransportError};

const HEADER_LAST_EVENT_ID: &str = "Last-Event-Id";
const EVENT_STREAM_MIME_TYPE: &str = "text/event-stream";

/// Error bodies are diagnostics, not payloads; cap what we retain.
pub(crate) const MAX_ERROR_BODY_BYTES: usize = 4096;

/// Builds an [`SseTransportError::HttpStatus`] from a non-success response,
/// capturing what `error_for_status` would discard: the `WWW-Authenticate`
/// challenge and a bounded copy of the body, both needed to classify auth
/// failures (e.g. the Warp proxy's `proxy_token_expired`).
async fn http_status_error<E: std::error::Error + Send + Sync + 'static>(
    response: reqwest::Response,
) -> SseTransportError<E> {
    let status = response.status();
    let www_authenticate = response
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut body = response.text().await.unwrap_or_default();
    if body.len() > MAX_ERROR_BODY_BYTES {
        let mut end = MAX_ERROR_BODY_BYTES;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        body.truncate(end);
    }
    SseTransportError::HttpStatus {
        status,
        body,
        www_authenticate,
    }
}

impl From<reqwest::Error> for SseTransportError<reqwest::Error> {
    fn from(e: reqwest::Error) -> Self {
        SseTransportError::Client(e)
    }
}

impl SseClient for reqwest::Client {
    type Error = reqwest::Error;

    async fn post_message(
        &self,
        uri: Uri,
        message: rmcp::model::ClientJsonRpcMessage,
        auth_token: Option<String>,
    ) -> Result<(), SseTransportError<Self::Error>> {
        let mut request_builder = self.post(uri.to_string()).json(&message);
        if let Some(auth_header) = auth_token {
            request_builder = request_builder.bearer_auth(auth_header);
        }
        let response = request_builder.send().await?;
        if !response.status().is_success() {
            return Err(http_status_error(response).await);
        }
        Ok(())
    }

    async fn get_stream(
        &self,
        uri: Uri,
        last_event_id: Option<String>,
        auth_token: Option<String>,
    ) -> Result<super::client_side_sse::BoxedSseResponse, SseTransportError<Self::Error>> {
        let mut request_builder = self
            .get(uri.to_string())
            .header(ACCEPT, EVENT_STREAM_MIME_TYPE);
        if let Some(auth_header) = auth_token {
            request_builder = request_builder.bearer_auth(auth_header);
        }
        if let Some(last_event_id) = last_event_id {
            request_builder = request_builder.header(HEADER_LAST_EVENT_ID, last_event_id);
        }
        let response = request_builder.send().await?;
        if !response.status().is_success() {
            return Err(http_status_error(response).await);
        }
        match response.headers().get(reqwest::header::CONTENT_TYPE) {
            Some(ct) => {
                if !ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) {
                    return Err(SseTransportError::UnexpectedContentType(Some(
                        String::from_utf8_lossy(ct.as_bytes()).to_string(),
                    )));
                }
            }
            None => {
                return Err(SseTransportError::UnexpectedContentType(None));
            }
        }
        let event_stream = SseStream::from_byte_stream(response.bytes_stream()).boxed();
        Ok(event_stream)
    }
}

impl SseClientTransport<reqwest::Client> {
    /// Creates a new transport using reqwest with the specified SSE endpoint.
    ///
    /// This is a convenience method that creates a transport using the default
    /// reqwest client.
    pub async fn start(
        uri: impl Into<Arc<str>>,
    ) -> Result<Self, SseTransportError<reqwest::Error>> {
        SseClientTransport::start_with_client(
            reqwest::Client::default(),
            SseClientConfig {
                sse_endpoint: uri.into(),
                ..Default::default()
            },
        )
        .await
    }
}
