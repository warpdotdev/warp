use std::result::Result as StdResult;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use instant::Duration;
use warp_graphql::client::RequestOptions;
use warp_server_auth::credentials::{AuthToken, Credentials, FirebaseToken, LoginToken};

use crate::auth::{AgentIdentity, UserAuthenticationError};

/// Application-provided capabilities used by extracted server API clients.
///
/// The base client keeps UI/model reactions and currently app-owned session
/// lifecycle plumbing outside extracted endpoint implementations.
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait BaseClient: Send + Sync {
    fn http_client(&self) -> Arc<http_client::Client>;

    fn anonymous_id(&self) -> String;

    fn unauthenticated_graphql_request_options(&self) -> RequestOptions;

    async fn graphql_request_options(&self, timeout: Option<Duration>) -> Result<RequestOptions>;

    async fn exchange_credentials(
        &self,
        token: LoginToken,
    ) -> StdResult<Credentials, UserAuthenticationError>;

    async fn get_or_refresh_access_token(&self) -> Result<AuthToken>;

    async fn request_device_code(
        &self,
    ) -> StdResult<oauth2::StandardDeviceAuthorizationResponse, UserAuthenticationError>;

    async fn exchange_device_access_token(
        &self,
        details: &oauth2::StandardDeviceAuthorizationResponse,
        timeout: Duration,
    ) -> StdResult<FirebaseToken, UserAuthenticationError>;

    async fn list_agent_identities(&self) -> Result<Vec<AgentIdentity>>;

    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>>;

    fn is_auth_refresh_allowed(&self) -> bool;

    fn on_graphql_staging_access_blocked(&self);

    fn on_graphql_iap_challenge_received(&self);

    fn on_graphql_user_account_disabled(&self);
}
