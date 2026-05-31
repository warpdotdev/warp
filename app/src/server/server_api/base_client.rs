use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use instant::Duration;
use warp_server_client::auth::AuthEvent;
use warp_server_client::base_client::BaseClient;

use super::ServerApi;
use crate::server::graphql::default_request_options;

/// Header key for the ambient workload token attached to multi-agent requests.
pub const AMBIENT_WORKLOAD_TOKEN_HEADER: &str = "X-Warp-Ambient-Workload-Token";

/// Header key for the cloud agent task ID attached to requests from ambient agents.
pub const CLOUD_AGENT_ID_HEADER: &str = "X-Warp-Cloud-Agent-ID";

/// Duration for which the ambient workload token is valid (3 hours).
const AMBIENT_WORKLOAD_TOKEN_DURATION: Duration = Duration::from_secs(3 * 60 * 60);

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl BaseClient for ServerApi {
    fn http_client(&self) -> Arc<http_client::Client> {
        self.client.clone()
    }

    fn anonymous_id(&self) -> String {
        ServerApi::anonymous_id(self)
    }

    fn unauthenticated_graphql_request_options(&self) -> warp_graphql::client::RequestOptions {
        default_request_options()
    }

    async fn graphql_request_options(
        &self,
        timeout: Option<Duration>,
    ) -> Result<warp_graphql::client::RequestOptions> {
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for GraphQL request")?;
        let mut headers = std::collections::HashMap::new();
        #[cfg(feature = "agent_mode_evals")]
        if let Some(eval_user_id) = self.eval_user_id {
            headers.insert(
                super::EVAL_USER_ID_HEADER.to_string(),
                eval_user_id.to_string(),
            );
        }
        for (name, value) in self.ambient_agent_headers().await? {
            headers.insert(name.to_string(), value);
        }
        Ok(warp_graphql::client::RequestOptions {
            auth_token: auth_token.bearer_token(),
            timeout,
            headers,
            ..default_request_options()
        })
    }

    async fn authenticated_public_api_request_headers(&self) -> Result<Vec<(String, String)>> {
        Ok(self
            .ambient_agent_headers()
            .await?
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect())
    }

    fn on_authenticated_public_api_failure(&self, response: &http_client::Response) {
        self.check_for_iap_challenge(response);
    }

    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>> {
        if cfg!(target_family = "wasm") {
            return Ok(None);
        }
        // Check whether a cached token remains valid, using a five-minute buffer.
        // Tokens without an expiration time are always considered valid.
        {
            let cached = self.ambient_workload_token.lock();
            if let Some(ref token) = *cached {
                let is_valid = token.expires_at.is_none_or(|expires_at| {
                    chrono::Utc::now() + chrono::Duration::minutes(5) < expires_at
                });
                if is_valid {
                    return Ok(Some(token.token.clone()));
                }
            }
        }
        // Issue a new token.
        let workload_token = match warp_isolation_platform::issue_workload_token(Some(
            AMBIENT_WORKLOAD_TOKEN_DURATION,
        ))
        .await
        {
            Ok(token) => token,
            Err(warp_isolation_platform::IsolationPlatformError::NoIsolationPlatformDetected) => {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let token = workload_token.token.clone();
        *self.ambient_workload_token.lock() = Some(workload_token);
        Ok(Some(token))
    }

    fn is_auth_refresh_allowed(&self) -> bool {
        self.allowed_to_refresh_token()
    }

    fn on_graphql_staging_access_blocked(&self) {
        let _ = self.event_sender.try_send(AuthEvent::StagingAccessBlocked);
    }

    fn on_graphql_iap_challenge_received(&self) {
        let _ = self.event_sender.try_send(AuthEvent::IapChallengeReceived);
    }

    fn on_graphql_user_account_disabled(&self) {
        let _ = self.event_sender.try_send(AuthEvent::UserAccountDisabled);
    }
}
