use anyhow::{Context as _, Result};
use serde::de::DeserializeOwned;
use warp_core::channel::ChannelState;
use warp_errors::{ErrorExt, register_error};

use crate::base_client::{AmbientHeaderPolicy, BaseClient};

/// Maximum number of bytes we will buffer from a public API JSON response before
/// giving up.
///
/// Public API endpoints return small, structured JSON payloads. A body larger
/// than this is a sign of a malformed or pathological response, and blindly
/// deserializing it can balloon into multiple gigabytes of heap as serde_json
/// expands the bytes into nested maps/values. Capping the raw body bounds that
/// blow-up (see Sentry issue 7259255054 / "Excessive memory usage detected").
const MAX_PUBLIC_API_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Typed error for HTTP operations so retry classifiers can inspect status failures.
#[derive(Debug, thiserror::Error)]
#[error("HTTP request failed with status {status}: {body}")]
pub struct HttpStatusError {
    pub status: u16,
    pub body: String,
}

impl ErrorExt for HttpStatusError {
    fn is_actionable(&self) -> bool {
        !matches!(self.status, 408 | 429)
    }
}

register_error!(HttpStatusError);

#[derive(serde::Deserialize)]
struct PublicApiError {
    error: String,
}

impl BaseClient {
    /// Sends a GET request to a public API endpoint and returns the raw response on success.
    ///
    /// Unlike [`get_public_api`], this does not attempt JSON deserialization on the
    /// response body, allowing the caller to decode it however they need.
    pub async fn get_public_api_response(&self, path: &str) -> Result<http_client::Response> {
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for API request")?;
        let url = format!("{}/api/v1/{path}", ChannelState::server_root_url());
        let mut request = self.http_client().get(&url);
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }
        for (name, value) in self
            .ambient_headers(AmbientHeaderPolicy::inherit_all())
            .await?
        {
            request = request.header(name, value);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to send API request to {url}"))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            self.observe_iap_challenge(&response);
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let status_error = HttpStatusError {
                status: status.as_u16(),
                body: body.clone(),
            };
            match serde_json::from_str::<PublicApiError>(&body) {
                Ok(error_response) => {
                    Err(anyhow::Error::new(status_error).context(error_response.error))
                }
                Err(_) => Err(anyhow::Error::new(status_error)
                    .context(format!("API request failed with status {status}"))),
            }
        }
    }

    /// Sends a GET request to a public API endpoint.
    ///
    /// # Arguments
    /// * `path` - Endpoint path relative to `/api/v1` (e.g., "agent/tasks/{task_id}")
    pub async fn get_public_api<R>(&self, path: &str) -> Result<R>
    where
        R: DeserializeOwned,
    {
        let response = self.get_public_api_response(path).await?;
        let response_url = response.url().clone();
        let body = read_body_bounded(response, MAX_PUBLIC_API_RESPONSE_BYTES)
            .await
            .with_context(|| format!("Failed to read response from {response_url}"))?;
        serde_json::from_slice::<R>(&body)
            .with_context(|| format!("Failed to deserialize response from {response_url}"))
    }
}

/// Returns the response's advertised body length from its `Content-Length` header,
/// if present and parseable.
fn advertised_content_length(response: &http_client::Response) -> Option<u64> {
    response
        .headers()
        .get(http::header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
}

/// Reads a response body into memory, failing fast if it exceeds `max_bytes`.
///
/// The advertised `Content-Length` is checked up front so an oversized response
/// is rejected before any large allocation. On native targets the body is then
/// streamed and accumulated with the same cap, so a chunked response that omits
/// (or lies about) its length still cannot grow the buffer past `max_bytes`.
async fn read_body_bounded(
    response: http_client::Response,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    if let Some(len) = advertised_content_length(&response) {
        anyhow::ensure!(
            len <= max_bytes as u64,
            "public API response body of {len} bytes exceeds maximum of {max_bytes} bytes"
        );
    }

    #[cfg(not(target_family = "wasm"))]
    let body = {
        use futures::StreamExt as _;

        let mut stream = response.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("Failed to read public API response chunk")?;
            anyhow::ensure!(
                buf.len() + chunk.len() <= max_bytes,
                "public API response body exceeds maximum of {max_bytes} bytes"
            );
            buf.extend_from_slice(&chunk);
        }
        buf
    };

    #[cfg(target_family = "wasm")]
    let body = {
        let bytes = response
            .bytes()
            .await
            .context("Failed to read public API response body")?;
        anyhow::ensure!(
            bytes.len() <= max_bytes,
            "public API response body exceeds maximum of {max_bytes} bytes"
        );
        bytes.to_vec()
    };

    Ok(body)
}

#[cfg(test)]
#[path = "public_api_tests.rs"]
mod tests;
