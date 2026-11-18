use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use http::HeaderValue;
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;
use warp_core::channel::ChannelState;

use super::{AMBIENT_WORKLOAD_TOKEN_HEADER, AmbientHeaderPolicy, BaseClient};

const MAX_RESPONSE_BYTES: usize = 4096;
static AUTHORIZATION_CLIENT: LazyLock<Result<reqwest::Client, reqwest::Error>> =
    LazyLock::new(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
    });

#[derive(Serialize)]
struct ViewerRequest<'a> {
    credential: &'a str,
    execution_id: i64,
    port: u16,
}

#[derive(Deserialize)]
struct ViewerResponse {
    allowed: bool,
    preview_id: Uuid,
    execution_id: i64,
    port: u16,
    valid_until: DateTime<Utc>,
}

impl BaseClient {
    /// Authorizes a viewer against the current run execution without caching the decision.
    pub async fn authorize_preview_viewer(
        &self,
        task_id: &str,
        execution_id: i64,
        preview_id: Uuid,
        port: u16,
        credential: &str,
    ) -> Result<()> {
        let root = Url::parse(ChannelState::server_root_url().as_ref())?;
        ensure!(
            root.scheme() == "https"
                || (root.scheme() == "http"
                    && matches!(root.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))),
            "Preview authorization requires a secure server URL"
        );
        ensure!(
            root.username().is_empty()
                && root.password().is_none()
                && root.query().is_none()
                && root.fragment().is_none(),
            "Invalid preview authorization server URL"
        );
        let mut url = root;
        url.path_segments_mut()
            .map_err(|()| anyhow::anyhow!("Invalid preview authorization server URL"))?
            .clear()
            .extend([
                "api",
                "v1",
                "agent",
                "runs",
                task_id,
                "previews",
                &preview_id.to_string(),
                "authorize",
            ]);
        let auth_token = self.get_or_refresh_access_token().await?;
        let worker_token = auth_token
            .as_bearer_token()
            .context("Preview authorization requires worker credentials")?;
        let ambient = self
            .ambient_headers(AmbientHeaderPolicy::for_task(task_id))
            .await?;
        ensure!(
            ambient.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case(AMBIENT_WORKLOAD_TOKEN_HEADER) && !value.is_empty()
            }),
            "Preview authorization requires a workload token"
        );
        let mut headers = reqwest::header::HeaderMap::new();
        let mut authorization = HeaderValue::from_str(&format!("Bearer {worker_token}"))?;
        authorization.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, authorization);
        for (name, value) in ambient {
            let mut value = HeaderValue::from_str(&value)?;
            value.set_sensitive(true);
            headers.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes())?,
                value,
            );
        }
        for (name, value) in self.iap_headers() {
            let mut value = HeaderValue::from_str(&value)?;
            value.set_sensitive(true);
            headers.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes())?,
                value,
            );
        }
        send_authorization(
            url,
            headers,
            preview_id,
            ViewerRequest {
                credential,
                execution_id,
                port,
            },
        )
        .await
    }
}

async fn send_authorization(
    url: Url,
    headers: reqwest::header::HeaderMap,
    preview_id: Uuid,
    body: ViewerRequest<'_>,
) -> Result<()> {
    let client = AUTHORIZATION_CLIENT
        .as_ref()
        .map_err(|_| anyhow::anyhow!("Preview authorization transport unavailable"))?;
    let response = client
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Preview authorization request failed"))?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "Viewer rejected"
    );
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| anyhow::anyhow!("Invalid preview authorization response"))?;
        ensure!(
            bytes.len() + chunk.len() <= MAX_RESPONSE_BYTES,
            "Authorization response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    let response: ViewerResponse = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid preview authorization response"))?;
    ensure!(
        response.allowed
            && response.preview_id == preview_id
            && response.execution_id == body.execution_id
            && response.port == body.port
            && response.valid_until > Utc::now(),
        "Viewer rejected"
    );
    Ok(())
}

#[cfg(test)]
#[path = "preview_authorization_tests.rs"]
mod tests;
