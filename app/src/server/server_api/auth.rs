use std::result::Result as StdResult;
use std::sync::Arc;

use anyhow::{bail, Context as _, Result};
use async_trait::async_trait;
use firebase::FetchAccessTokenResponse;
use futures::FutureExt;
use instant::Duration;
use oauth2::TokenResponse;
use thiserror::Error;
use warp_graphql::queries::get_user::UserOutput as GqlUserOutput;
#[cfg(test)]
pub use warp_server_client::auth::MockAuthClient;
pub use warp_server_client::auth::{
    AgentIdentity, AuthClient, AuthClientImpl, FetchUserResult, MintCustomTokenError,
    SyncedUserSettings, UserAuthenticationError,
};
use warp_server_client::base_client::BaseClient;
use warpui::r#async::BoxFuture;

use super::ServerApi;
use crate::auth::credentials::{AuthToken, Credentials, FirebaseToken, LoginToken, RefreshToken};
use crate::auth::user::{FirebaseAuthTokens, User};
use crate::auth::UserUid;
use crate::channel::ChannelState;
use crate::convert_to_server_experiment;
use crate::server::experiments::ServerExperiment;
use crate::server::graphql::default_request_options;
use crate::server::server_api::ServerApiEvent;

/// Wrapper for the `GET /api/v1/agent/identities` response.
#[derive(serde::Deserialize)]
struct AgentIdentitiesResponse {
    agents: Vec<AgentIdentity>,
}

const FETCH_ACCESS_TOKEN_TIMEOUT: Duration = Duration::from_secs(5);

/// Header key for the ambient workload token attached to multi-agent requests.
pub const AMBIENT_WORKLOAD_TOKEN_HEADER: &str = "X-Warp-Ambient-Workload-Token";

/// Header key for the cloud agent task ID attached to requests from ambient agents.
pub const CLOUD_AGENT_ID_HEADER: &str = "X-Warp-Cloud-Agent-ID";

/// Duration for which the ambient workload token is valid (3 hours).
const AMBIENT_WORKLOAD_TOKEN_DURATION: Duration = Duration::from_secs(3 * 60 * 60);

impl ServerApi {
    pub(super) async fn access_token(&self) -> Result<AuthToken> {
        if cfg!(feature = "skip_login") {
            bail!("skip_login enabled; failing all authenticated requests");
        }

        let Some(credentials) = self.auth_state.credentials() else {
            bail!("missing authentication credentials");
        };

        match credentials {
            Credentials::ApiKey { key, .. } => Ok(AuthToken::ApiKey(key)),
            Credentials::Bearer(token) => Ok(AuthToken::Bearer(token)),
            Credentials::Firebase(auth_tokens) => {
                let expiration_time = auth_tokens.expiration_time;

                // Generate a new ID token if the token has expired or will expire in the
                // next five minutes. This matches the behavior of the Firebase Auth SDK.
                if chrono::Local::now().fixed_offset() + chrono::Duration::minutes(5)
                    >= expiration_time
                {
                    let refresh_token = auth_tokens.refresh_token.clone();
                    let firebase_token = FirebaseToken::Refresh(RefreshToken::new(refresh_token));

                    let result = fetch_auth_tokens(self.client.clone(), firebase_token).await;

                    if let Err(UserAuthenticationError::DeniedAccessToken(_)) = result {
                        let _ = self.event_sender.send(ServerApiEvent::NeedsReauth).await;
                    }
                    let new_firebase_token_info = result?;
                    self.auth_state
                        .update_firebase_tokens(new_firebase_token_info.clone());
                    let _ = self
                        .event_sender
                        .send(ServerApiEvent::AccessTokenRefreshed {
                            token: new_firebase_token_info.id_token.clone(),
                        })
                        .await;
                    return Ok(AuthToken::Firebase(new_firebase_token_info.id_token));
                }

                Ok(AuthToken::Firebase(auth_tokens.id_token))
            }
            Credentials::SessionCookie => Ok(AuthToken::NoAuth),
            #[cfg(any(feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => Ok(AuthToken::NoAuth),
        }
    }
}

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
            .access_token()
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

    async fn exchange_credentials(
        &self,
        token: LoginToken,
    ) -> StdResult<Credentials, UserAuthenticationError> {
        exchange_credentials(self.client.clone(), token).await
    }

    async fn get_or_refresh_access_token(&self) -> Result<AuthToken> {
        self.access_token().await
    }

    async fn request_device_code(
        &self,
    ) -> StdResult<oauth2::StandardDeviceAuthorizationResponse, UserAuthenticationError> {
        self.oauth_client
            .exchange_device_code()
            .request_async(self.client.as_ref())
            .await
            .context("Failed to generate device code")
            .map_err(UserAuthenticationError::Unexpected)
    }

    async fn exchange_device_access_token(
        &self,
        details: &oauth2::StandardDeviceAuthorizationResponse,
        timeout: Duration,
    ) -> StdResult<FirebaseToken, UserAuthenticationError> {
        let result = self
            .oauth_client
            .exchange_device_access_token(details)
            .request_async(
                self.client.as_ref(),
                |delay| warpui::r#async::Timer::after(delay).map(|_| ()),
                Some(timeout),
            )
            .await
            .context("Unable to obtain access token")
            .map_err(UserAuthenticationError::Unexpected)?;
        Ok(FirebaseToken::Custom(
            result.access_token().secret().to_string(),
        ))
    }

    async fn list_agent_identities(&self) -> Result<Vec<AgentIdentity>> {
        let response: AgentIdentitiesResponse = self.get_public_api("agent/identities").await?;
        Ok(response.agents)
    }

    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>> {
        if cfg!(target_family = "wasm") {
            return Ok(None);
        }
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
        let _ = self
            .event_sender
            .try_send(ServerApiEvent::StagingAccessBlocked);
    }

    fn on_graphql_iap_challenge_received(&self) {
        let _ = self
            .event_sender
            .try_send(ServerApiEvent::IapChallengeReceived);
    }

    fn on_graphql_user_account_disabled(&self) {
        let _ = self
            .event_sender
            .try_send(ServerApiEvent::UserAccountDisabled);
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl AuthClient for ServerApi {
    async fn create_anonymous_user(
        &self,
        referral_code: Option<String>,
        anonymous_user_type: warp_graphql::mutations::create_anonymous_user::AnonymousUserType,
    ) -> Result<warp_graphql::mutations::create_anonymous_user::CreateAnonymousUserResult> {
        AuthClientImpl::new(self)
            .create_anonymous_user(referral_code, anonymous_user_type)
            .await
    }

    async fn get_or_refresh_access_token(&self) -> Result<AuthToken> {
        AuthClientImpl::new(self)
            .get_or_refresh_access_token()
            .await
    }

    async fn fetch_user(
        &self,
        token: LoginToken,
        for_refresh: bool,
    ) -> StdResult<FetchUserResult, UserAuthenticationError> {
        AuthClientImpl::new(self)
            .fetch_user(token, for_refresh)
            .await
    }

    async fn fetch_new_custom_token(
        &self,
    ) -> Result<warp_graphql::mutations::mint_custom_token::MintCustomTokenResult> {
        AuthClientImpl::new(self).fetch_new_custom_token().await
    }

    fn on_custom_token_fetched(
        &self,
        response: Result<warp_graphql::mutations::mint_custom_token::MintCustomTokenResult>,
    ) -> Result<String, MintCustomTokenError> {
        AuthClientImpl::new(self).on_custom_token_fetched(response)
    }

    async fn fetch_user_properties<'a>(
        &self,
        auth_token: Option<&'a str>,
    ) -> Result<GqlUserOutput> {
        AuthClientImpl::new(self)
            .fetch_user_properties(auth_token)
            .await
    }

    async fn get_user_settings(&self) -> Result<Option<SyncedUserSettings>> {
        AuthClientImpl::new(self).get_user_settings().await
    }

    async fn set_is_telemetry_enabled(&self, value: bool) -> Result<()> {
        AuthClientImpl::new(self)
            .set_is_telemetry_enabled(value)
            .await
    }

    async fn set_is_crash_reporting_enabled(&self, value: bool) -> Result<()> {
        AuthClientImpl::new(self)
            .set_is_crash_reporting_enabled(value)
            .await
    }

    async fn set_is_cloud_conversation_storage_enabled(&self, value: bool) -> Result<()> {
        AuthClientImpl::new(self)
            .set_is_cloud_conversation_storage_enabled(value)
            .await
    }

    async fn update_user_settings(
        &self,
        input: warp_graphql::mutations::update_user_settings::UpdateUserSettingsInput,
    ) -> Result<()> {
        AuthClientImpl::new(self).update_user_settings(input).await
    }

    async fn set_user_is_onboarded(&self) -> Result<bool> {
        AuthClientImpl::new(self).set_user_is_onboarded().await
    }

    async fn request_device_code(
        &self,
    ) -> StdResult<oauth2::StandardDeviceAuthorizationResponse, UserAuthenticationError> {
        AuthClientImpl::new(self).request_device_code().await
    }

    async fn exchange_device_access_token(
        &self,
        details: &oauth2::StandardDeviceAuthorizationResponse,
        timeout: Duration,
    ) -> StdResult<FirebaseToken, UserAuthenticationError> {
        AuthClientImpl::new(self)
            .exchange_device_access_token(details, timeout)
            .await
    }

    async fn list_api_keys(
        &self,
    ) -> Result<Vec<warp_graphql::queries::api_keys::ApiKeyProperties>> {
        AuthClientImpl::new(self).list_api_keys().await
    }

    async fn create_api_key(
        &self,
        name: String,
        team_id: Option<cynic::Id>,
        agent_uid: Option<cynic::Id>,
        expires_at: Option<warp_graphql::scalars::Time>,
    ) -> Result<warp_graphql::mutations::generate_api_key::GenerateApiKeyResult> {
        AuthClientImpl::new(self)
            .create_api_key(name, team_id, agent_uid, expires_at)
            .await
    }

    async fn expire_api_key(
        &self,
        key_uid: &crate::server::ids::ApiKeyUid,
    ) -> Result<warp_graphql::mutations::expire_api_key::ExpireApiKeyResult> {
        AuthClientImpl::new(self).expire_api_key(key_uid).await
    }

    async fn list_agent_identities(&self) -> Result<Vec<AgentIdentity>> {
        AuthClientImpl::new(self).list_agent_identities().await
    }

    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>> {
        AuthClientImpl::new(self)
            .get_or_create_ambient_workload_token()
            .await
    }
}

/// Exchange a long-lived token for fresh [`Credentials`].
async fn exchange_credentials(
    client: Arc<http_client::Client>,
    token: LoginToken,
) -> StdResult<Credentials, UserAuthenticationError> {
    match token {
        LoginToken::Firebase(firebase_token) => {
            let tokens = fetch_auth_tokens(client, firebase_token).await?;
            Ok(Credentials::Firebase(tokens))
        }
        LoginToken::ApiKey(key) => Ok(Credentials::ApiKey {
            key,
            owner_type: None,
        }),
        LoginToken::SessionCookie => Ok(Credentials::SessionCookie),
    }
}

fn fetch_auth_tokens(
    client: Arc<http_client::Client>,
    token: FirebaseToken,
) -> BoxFuture<'static, StdResult<FirebaseAuthTokens, UserAuthenticationError>> {
    Box::pin(async move {
        let firebase_api_key = ChannelState::firebase_api_key();
        let url = token.access_token_url(&firebase_api_key);
        let request_body = token.access_token_request_body();
        let proxy_url = token.proxy_url(&ChannelState::server_root_url(), &firebase_api_key);
        let response = match client
            .post(&url)
            .form(&request_body)
            .timeout(FETCH_ACCESS_TOKEN_TIMEOUT)
            .send()
            .await
        {
            Ok(response) => match response.error_for_status_ref() {
                Ok(_) => Ok(response),
                Err(error) => {
                    log::warn!(
                        "Request to firebase to fetch access token completed, but was unsuccessful: {error:?}"
                    );

                    fetch_access_token_via_proxy(client, &request_body, proxy_url).await
                }
            },
            Err(error) => {
                log::warn!("Failed to make response to firebase to fetch access token: {error:?}");

                fetch_access_token_via_proxy(client, &request_body, proxy_url).await
            }
        }?;

        let response = response
            .json::<FetchAccessTokenResponse>()
            .await
            .map_err(anyhow::Error::from)?;
        match response {
            FetchAccessTokenResponse::Success {
                id_token,
                expires_in,
                refresh_token,
            } => Ok(FirebaseAuthTokens::from_response(
                id_token,
                refresh_token,
                expires_in,
            )?),
            FetchAccessTokenResponse::Error { error } => Err(error.into()),
        }
    })
}

fn fetch_access_token_via_proxy<'a>(
    client: Arc<http_client::Client>,
    request_body: &'a [(&'a str, &'a str)],
    proxy_url: String,
) -> BoxFuture<'a, Result<http_client::Response>> {
    Box::pin(async move {
        client
            .post(&proxy_url)
            .form(request_body)
            .send()
            .await
            .map_err(anyhow::Error::from)
    })
}

/// The [`oauth2::Client`] type, specialized to the endpoints that we require.
pub type OAuth2Client = oauth2::basic::BasicClient<
    oauth2::EndpointNotSet, // HasAuthUrl
    oauth2::EndpointSet,    // HasDeviceAuthUrl
    oauth2::EndpointNotSet, // HasIntrospectionUrl
    oauth2::EndpointNotSet, // HasRevocationUrl
    oauth2::EndpointSet,    // HasTokenUrl
>;

/// Intermediate type produced by converting a [`GqlUserOutput`] from the server.
pub(crate) struct UserProperties {
    pub(crate) user: User,
    pub(crate) server_experiments: Vec<ServerExperiment>,
    pub(crate) llms: crate::ai::llms::ModelsByFeature,
}

impl From<GqlUserOutput> for UserProperties {
    fn from(user_output: GqlUserOutput) -> Self {
        let principal_type = user_output
            .principal_type
            .map(|pt| pt.into())
            .unwrap_or_default();
        let user_properties = user_output.user;

        let is_on_work_domain = user_properties.is_on_work_domain;
        let is_onboarded = user_properties.is_onboarded;
        let global_skills = user_properties.global_skills;

        let linked_at = user_properties
            .anonymous_user_info
            .as_ref()
            .and_then(|info| info.linked_at);

        let anonymous_user_type = user_properties
            .anonymous_user_info
            .as_ref()
            .map(|info| info.anonymous_user_type.clone());
        let personal_object_limits = user_properties
            .anonymous_user_info
            .and_then(|info| info.personal_object_limits.clone());
        let user_profile = user_properties.profile;
        let local_id = UserUid::new(user_profile.uid.as_str());
        let needs_sso_link = user_profile.needs_sso_link;

        let server_experiments: Vec<ServerExperiment> = user_properties
            .experiments
            .and_then(|experiments| convert_to_server_experiment!(experiments))
            .unwrap_or_default();

        // Convert LLM model choices from GraphQL response
        let llms = user_properties.llms.try_into().unwrap_or_default();

        let user = User {
            is_onboarded,
            local_id,
            metadata: user_profile.into(),
            needs_sso_link,
            anonymous_user_type: anonymous_user_type.and_then(|t| t.try_into().ok()),
            is_on_work_domain,
            linked_at,
            personal_object_limits: personal_object_limits.and_then(|t| t.try_into().ok()),
            principal_type,
            global_skills,
        };

        UserProperties {
            user,
            server_experiments,
            llms,
        }
    }
}

#[derive(Error, Debug)]
/// Error type when creating anonymous users
pub enum AnonymousUserCreationError {
    #[error("The network request to create the anonymous user failed")]
    CreationFailed,

    #[error("Received a user facing error: {0}")]
    UserFacingError(String),

    /// Failure that occurs after the user is created, but the ID token could not be fetched.
    #[error("The user was created, but the ID token could not be fetched")]
    UserAuthenticationFailed(#[from] UserAuthenticationError),

    #[error("Failed to create anonymous user with unknown error")]
    Unknown,
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
