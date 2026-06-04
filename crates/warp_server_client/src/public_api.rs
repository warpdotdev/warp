use anyhow::{Context as _, Result};
use serde::de::DeserializeOwned;
use warp_core::channel::ChannelState;
use warp_core::errors::{ErrorExt, register_error};

use crate::base_client::{AmbientHeaderPolicy, BaseClient};

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
        response
            .json::<R>()
            .await
            .with_context(|| format!("Failed to deserialize response from {response_url}"))
    }
}

#[cfg(test)]
#[path = "public_api_tests.rs"]
mod tests;
