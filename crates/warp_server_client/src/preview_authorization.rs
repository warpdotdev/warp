use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use http::HeaderValue;
use serde::Serialize;
use url::Url;
use uuid::Uuid;
use warp_core::channel::ChannelState;

use super::{AMBIENT_WORKLOAD_TOKEN_HEADER, AmbientHeaderPolicy, BaseClient};

static AUTHORIZATION_CLIENT: LazyLock<Result<reqwest::Client, reqwest::Error>> =
    LazyLock::new(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
    });

#[derive(Serialize)]
struct IngressRequest<'a> {
    ingress_token: &'a str,
    execution_id: i64,
    port: u16,
}

impl BaseClient {
    /// Validates a server-issued ingress token against the current execution without caching.
    pub async fn validate_preview_ingress_token(
        &self,
        task_id: &str,
        execution_id: i64,
        preview_id: Uuid,
        port: u16,
        ingress_token: &str,
    ) -> Result<()> {
        let root = Url::parse(ChannelState::server_root_url().as_ref())?;
        ensure!(
            root.scheme() == "https"
                || (root.scheme() == "http"
                    && matches!(root.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))),
            "Preview ingress validation requires a secure server URL"
        );
        ensure!(
            root.username().is_empty()
                && root.password().is_none()
                && root.query().is_none()
                && root.fragment().is_none(),
            "Invalid preview ingress validation server URL"
        );
        let mut url = root;
        url.path_segments_mut()
            .map_err(|()| anyhow::anyhow!("Invalid preview ingress validation server URL"))?
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
            .context("Preview ingress validation requires worker credentials")?;
        let ambient = self
            .ambient_headers(AmbientHeaderPolicy::for_task(task_id))
            .await?;
        ensure!(
            ambient.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case(AMBIENT_WORKLOAD_TOKEN_HEADER) && !value.is_empty()
            }),
            "Preview ingress validation requires a workload token"
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
        send_ingress_validation(
            url,
            headers,
            IngressRequest {
                ingress_token,
                execution_id,
                port,
            },
        )
        .await
    }
}

async fn send_ingress_validation(
    url: Url,
    headers: reqwest::header::HeaderMap,
    body: IngressRequest<'_>,
) -> Result<()> {
    let client = AUTHORIZATION_CLIENT
        .as_ref()
        .map_err(|_| anyhow::anyhow!("Preview ingress validation transport unavailable"))?;
    let response = client
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Preview ingress validation request failed"))?;
    ensure!(
        response.status() == reqwest::StatusCode::NO_CONTENT,
        "Ingress token rejected"
    );
    Ok(())
}

#[cfg(test)]
#[path = "preview_authorization_tests.rs"]
mod tests;
