use anyhow::{Context as _, Result};
use serde::de::DeserializeOwned;
use warp_core::channel::ChannelState;
use warp_server_auth::credentials::AuthToken;

use crate::base_client::BaseClient;

/// Typed error for HTTP-backed operations so downstream classifiers can distinguish
/// transient and permanent status failures without parsing error text.
#[derive(Debug, thiserror::Error)]
#[error("HTTP request failed with status {status}: {body}")]
pub struct HttpStatusError {
    pub status: u16,
    pub body: String,
}

#[derive(serde::Deserialize)]
struct PublicApiError {
    error: String,
}

/// Sends an authenticated GET request to a public API endpoint and deserializes its response.
pub(crate) async fn get_authenticated_public_api<R>(
    base_client: &dyn BaseClient,
    auth_token: AuthToken,
    path: &str,
) -> Result<R>
where
    R: DeserializeOwned,
{
    let url = format!("{}/api/v1/{path}", ChannelState::server_root_url());
    let http_client = base_client.http_client();
    let mut request = http_client.get(&url);
    if let Some(token) = auth_token.as_bearer_token() {
        request = request.bearer_auth(token);
    }

    for (name, value) in base_client
        .authenticated_public_api_request_headers()
        .await?
    {
        request = request.header(name, value);
    }

    let response = request
        .send()
        .await
        .with_context(|| format!("Failed to send API request to {url}"))?;

    if !response.status().is_success() {
        base_client.on_authenticated_public_api_failure(&response);
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let status_error = HttpStatusError {
            status: status.as_u16(),
            body: body.clone(),
        };
        return match serde_json::from_str::<PublicApiError>(&body) {
            Ok(error_response) => {
                Err(anyhow::Error::new(status_error).context(error_response.error))
            }
            Err(_) => Err(anyhow::Error::new(status_error)
                .context(format!("API request failed with status {status}"))),
        };
    }

    let response_url = response.url().clone();
    response
        .json::<R>()
        .await
        .with_context(|| format!("Failed to deserialize response from {response_url}"))
}

#[cfg(test)]
#[path = "public_api_tests.rs"]
mod tests;
