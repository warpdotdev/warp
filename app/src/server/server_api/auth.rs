use std::{result::Result as StdResult, sync::Arc};

use anyhow::{anyhow, bail, Context as _, Result};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
use firebase::{FetchAccessTokenResponse, FirebaseError};
use futures::FutureExt;
use instant::Duration;
#[cfg(test)]
use mockall::{automock, predicate::*};
use oauth2::TokenResponse;
use thiserror::Error;
use warp_core::errors::{AnyhowErrorExt, ErrorExt};
use warp_graphql::client::Operation;
use warp_graphql::mutations::expire_api_key::{
    ExpireApiKey, ExpireApiKeyResult, ExpireApiKeyVariables,
};
use warp_graphql::queries::get_conversation_usage::{
    ConversationUsage, GetConversationUsage, GetConversationUsageVariables, UserResult,
};

use warp_graphql::mutations::set_user_is_onboarded::{
    SetUserIsOnboarded, SetUserIsOnboardedResult, SetUserIsOnboardedVariables,
};
use warp_graphql::mutations::update_user_settings::{
    UpdateUserSettings, UpdateUserSettingsInput, UpdateUserSettingsResult,
    UpdateUserSettingsVariables,
};
use warp_graphql::mutations::{
    create_anonymous_user::{
        AnonymousUserType, CreateAnonymousUser, CreateAnonymousUserResult,
        CreateAnonymousUserVariables,
    },
    generate_api_key::{
        GenerateApiKey, GenerateApiKeyInput, GenerateApiKeyResult, GenerateApiKeyVariables,
    },
    mint_custom_token::{MintCustomTokenResult, MintCustomTokenVariables},
};
use warp_graphql::object_permissions::OwnerType;
use warp_graphql::queries::api_keys::{
    ApiKeyProperties, ApiKeyPropertiesResult, ApiKeys, ApiKeysVariables,
};
use warp_graphql::queries::get_user::{GetUser, GetUserVariables, UserOutput as GqlUserOutput};
use warp_graphql::queries::get_user_settings::{GetUserSettings, GetUserSettingsVariables};
use warpui::r#async::BoxFuture;

use crate::auth::UserUid;
use crate::server::graphql::{default_request_options, get_user_facing_error_message};
use crate::server::ids::ApiKeyUid;
use crate::server::server_api::register_error;
use crate::server::server_api::EXPERIMENT_ID_HEADER;
use crate::settings::PrivacySettingsSnapshot;
use crate::{
    auth::{
        credentials::{AuthToken, Credentials, FirebaseToken, LoginToken, RefreshToken},
        user::FirebaseAuthTokens,
        user::User,
    },
    channel::ChannelState,
    convert_to_server_experiment,
    server::{
        datetime_ext::DateTimeExt as _, experiments::ServerExperiment,
        graphql::get_request_context, server_api::ServerApiEvent,
    },
};

use super::ServerApi;

/// A named agent identity from the public API.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AgentIdentity {
    pub uid: String,
    pub name: String,
    pub available: bool,
}

/// Wrapper for the `GET /api/v1/agent/identities` response.
#[derive(serde::Deserialize)]
struct AgentIdentitiesResponse {
    agents: Vec<AgentIdentity>,
}

/// Error messages returned from the Firebase REST API when attempting to convert a refresh token
/// into an access token that indicate the user's token is in an errored state.
/// These are "soft" errors because the user likely just needs to log in again.
/// See https://firebase.google.com/docs/reference/rest/auth#section-refresh-token.
static FETCH_ACCESS_TOKEN_SOFT_ERROR_MESSAGES: &[&str] = &[
    "TOKEN_EXPIRED",
    "INVALID_REFRESH_TOKEN",
    "MISSING_REFRESH_TOKEN",
];

/// Error messages returned from the Firebase REST API when attempting to convert a refresh token
/// into an access token that indicate the user's account is in an errored state.
/// These are "hard" errors because the user likely can no longer sign in with their account,
/// for example if it were disabled or deleted.
/// See https://firebase.google.com/docs/reference/rest/auth#section-refresh-token.
static FETCH_ACCESS_TOKEN_HARD_ERROR_MESSAGES: &[&str] = &["USER_DISABLED", "USER_NOT_FOUND"];

const FETCH_ACCESS_TOKEN_TIMEOUT: Duration = Duration::from_secs(5);

/// Header key for the ambient workload token attached to multi-agent requests.
pub const AMBIENT_WORKLOAD_TOKEN_HEADER: &str = "X-Warp-Ambient-Workload-Token";

/// Header key for the cloud agent task ID attached to requests from ambient agents.
pub const CLOUD_AGENT_ID_HEADER: &str = "X-Warp-Cloud-Agent-ID";

/// Duration for which the ambient workload token is valid (3 hours).
const AMBIENT_WORKLOAD_TOKEN_DURATION: Duration = Duration::from_secs(3 * 60 * 60);

/// User settings that are currently 'synced' (e.g. stored server-side) on a per-user basis.
#[derive(Copy, Clone, Debug, Default)]
pub struct SyncedUserSettings {
    pub is_cloud_conversation_storage_enabled: bool,
    pub is_crash_reporting_enabled: bool,
    pub is_telemetry_enabled: bool,
}

/// Results of an attempt to fetch the current user.
pub struct FetchUserResult {
    pub user: User,
    /// The credentials used to authenticate this user.
    pub credentials: Credentials,
    pub server_experiments: Vec<ServerExperiment>,
    /// Whether this attempt to fetch the user was for refreshing an existing logged-in user.
    pub from_refresh: bool,
    /// LLM model choices for this user.
    pub llms: crate::ai::llms::ModelsByFeature,
}

#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait AuthClient: 'static + Send + Sync {
    /// Creates an anonymous user, who is allowed to use Warp but may lack the ability
    /// to interact with particular features.
    async fn create_anonymous_user(
        &self,
        referral_code: Option<String>,
        anonymous_user_type: AnonymousUserType,
    ) -> Result<CreateAnonymousUserResult>;

    /// Returns the cached access token, if it is still valid. If it has expired, fetches a new
    /// access token using the user's refresh token, caches it, and the returns it.
    /// Returns an auth mode that may not require an Authorization header (e.g. session cookies or
    /// test credentials).
    async fn get_or_refresh_access_token(&self) -> Result<AuthToken>;

    /// Fetches data required to construct the [`User`] object. This includes the user's metadata
    /// and authentication tokens.
    async fn fetch_user(
        &self,
        token: LoginToken,
        for_refresh: bool,
    ) -> StdResult<FetchUserResult, UserAuthenticationError>;

    /// Creates and fetches an new custom token for the current user from Firebase.
    /// This only works for anonymous users, and will surface an error if the user is not anonymous.
    async fn fetch_new_custom_token(&self) -> Result<MintCustomTokenResult>;

    /// Handles the response from [`Self::fetch_new_custom_token`], returning the newly-minted custom token.
    fn on_custom_token_fetched(
        &self,
        response: Result<MintCustomTokenResult>,
    ) -> Result<String, MintCustomTokenError>;

    /// Queries warp-server for a set of the currently logged-in user's fields.
    async fn fetch_user_properties<'a>(&self, auth_token: Option<&'a str>)
        -> Result<GqlUserOutput>;

    /// Upon success, returns an `Option` containing the user's settings retrieved from the server,
    /// if any. The user may not have server-side settings if they onboarded prior to the launch
    /// of telemetry opt-out, have not logged in since the launch, and have never changed defaults
    /// for any of the settings in [`SyncedUserSettings`]. If the fetched settings object exists
    /// but is missing required fields, or if the request itself failed, returns an error.
    async fn get_user_settings(&self) -> Result<Option<SyncedUserSettings>>;

    /// Returns conversation usage history for the current user over the past n days.
    /// If last_updated_end_timestamp is provided, only conversations with
    /// lastUpdated earlier than this timestamp are returned.
    async fn get_conversation_usage_history(
        &self,
        days: Option<i32>,
        limit: Option<i32>,
        last_updated_end_timestamp: Option<warp_graphql::scalars::Time>,
    ) -> Result<Vec<ConversationUsage>>;

    async fn set_is_telemetry_enabled(&self, value: bool) -> Result<()>;

    async fn set_is_crash_reporting_enabled(&self, value: bool) -> Result<()>;

    async fn set_is_cloud_conversation_storage_enabled(&self, value: bool) -> Result<()>;

    /// Sends a request to update the user's settings on the server with values contained in the
    /// given `settings_snapshot`.
    async fn update_user_settings(&self, settings_snapshot: PrivacySettingsSnapshot) -> Result<()>;

    async fn set_user_is_onboarded(&self) -> Result<bool>;

    /// Requests a device authorization code from the server. This is only used for headless CLI/SDK authentication.
    async fn request_device_code(
        &self,
    ) -> StdResult<oauth2::StandardDeviceAuthorizationResponse, UserAuthenticationError>;

    /// Wait for the request to be approved or rejected and exchange it for a short-lived custom access token.
    async fn exchange_device_access_token(
        &self,
        details: &oauth2::StandardDeviceAuthorizationResponse,
        timeout: Duration,
    ) -> StdResult<FirebaseToken, UserAuthenticationError>;
    // API Keys
    async fn list_api_keys(&self) -> Result<Vec<ApiKeyProperties>>;

    async fn create_api_key(
        &self,
        name: String,
        team_id: Option<cynic::Id>,
        agent_uid: Option<cynic::Id>,
        expires_at: Option<warp_graphql::scalars::Time>,
    ) -> Result<GenerateApiKeyResult>;

    async fn expire_api_key(&self, key_uid: &ApiKeyUid) -> Result<ExpireApiKeyResult>;

    /// Fetches the list of named agent identities for the user's team.
    async fn list_agent_identities(&self) -> Result<Vec<AgentIdentity>>;

    /// Returns a cached ambient workload token, or issues a new one if not present or expired.
    ///
    /// Returns `Ok(None)` if not running in an isolation platform (e.g., Namespace) or on WASM.
    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>>;
}

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
                if chrono::DateTime::now() + chrono::Duration::minutes(5) >= expiration_time {
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
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => Ok(AuthToken::NoAuth),
        }
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl AuthClient for ServerApi {
    async fn create_anonymous_user(
        &self,
        referral_code: Option<String>,
        anonymous_user_type: AnonymousUserType,
    ) -> Result<CreateAnonymousUserResult> {
        let variables = CreateAnonymousUserVariables {
            input: warp_graphql::mutations::create_anonymous_user::CreateAnonymousUserInput {
                anonymous_user_type,
                expiration_type: warp_graphql::mutations::create_anonymous_user::AnonymousUserExpirationType::NoExpiration,
                referral_code,
            },
            request_context: get_request_context(),
        };

        let operation = CreateAnonymousUser::build(variables);
        let response = operation
            .send_request(self.client.clone(), default_request_options())
            .await?;

        Ok(response
            .data
            .ok_or_else(|| anyhow!("missing data in response"))?
            .create_anonymous_user)
    }

    async fn get_or_refresh_access_token(&self) -> Result<AuthToken> {
        self.access_token().await
    }

    async fn fetch_user(
        &self,
        token: LoginToken,
        for_refresh: bool,
    ) -> StdResult<FetchUserResult, UserAuthenticationError> {
        let new_credentials = exchange_credentials(self.client.clone(), token).await?;
        let auth_token = new_credentials.bearer_token();
        let user_output = self
            .fetch_user_properties(auth_token.as_bearer_token())
            .await
            .context("Failed to fetch user response data")
            .map_err(UserAuthenticationError::Unexpected)?;

        let UserProperties {
            user,
            server_experiments,
            llms,
            api_key_owner_type,
        } = user_output.into();

        // Store the owner type if using an API key.
        let new_credentials = match new_credentials {
            Credentials::ApiKey { key, .. } => Credentials::ApiKey {
                key,
                owner_type: api_key_owner_type,
            },
            other => other,
        };

        Ok(FetchUserResult {
            user,
            credentials: new_credentials,
            server_experiments,
            from_refresh: for_refresh,
            llms,
        })
    }

    async fn fetch_new_custom_token(&self) -> Result<MintCustomTokenResult> {
        let variables = MintCustomTokenVariables {
            request_context: get_request_context(),
        };

        let operation =
            warp_graphql::mutations::mint_custom_token::MintCustomToken::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        Ok(response.mint_custom_token)
    }

    fn on_custom_token_fetched(
        &self,
        response: Result<MintCustomTokenResult>,
    ) -> Result<String, MintCustomTokenError> {
        match response {
            Ok(response_data) => match response_data {
                MintCustomTokenResult::MintCustomTokenOutput(output) => Ok(output.custom_token),
                MintCustomTokenResult::UserFacingError(user_facing_error) => {
                    Err(MintCustomTokenError::UserFacingError(
                        get_user_facing_error_message(user_facing_error),
                    ))
                }
                MintCustomTokenResult::Unknown => Err(MintCustomTokenError::Unknown),
            },
            Err(_) => Err(MintCustomTokenError::Unknown),
        }
    }

    async fn fetch_user_properties<'a>(
        &self,
        auth_token: Option<&'a str>,
    ) -> Result<GqlUserOutput> {
        let variables = GetUserVariables {
            request_context: get_request_context(),
        };
        let operation = GetUser::build(variables);
        let response = operation
            .send_request(
                self.client.clone(),
                warp_graphql::client::RequestOptions {
                    auth_token: auth_token.map(ToOwned::to_owned),
                    headers: std::collections::HashMap::from([(
                        EXPERIMENT_ID_HEADER.to_string(),
                        self.anonymous_id(),
                    )]),
                    ..default_request_options()
                },
            )
            .await?
            .data
            .ok_or_else(|| anyhow!("Expected valid response.data"))?;

        match response.user {
            warp_graphql::queries::get_user::UserResult::UserOutput(user_output) => Ok(user_output),
            warp_graphql::queries::get_user::UserResult::Unknown => {
                Err(anyhow!("Unable to fetch user"))
            }
        }
    }

    async fn get_user_settings(&self) -> Result<Option<SyncedUserSettings>> {
        let variables = GetUserSettingsVariables {
            request_context: get_request_context(),
        };
        let operation = GetUserSettings::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.user {
            warp_graphql::queries::get_user_settings::UserResult::UserOutput(user_output) => {
                match user_output.user.settings {
                    Some(user_settings) => Ok(Some(SyncedUserSettings {
                        is_cloud_conversation_storage_enabled: user_settings
                            .is_cloud_conversation_storage_enabled,
                        is_crash_reporting_enabled: user_settings.is_crash_reporting_enabled,
                        is_telemetry_enabled: user_settings.is_telemetry_enabled,
                    })),
                    None => Ok(None),
                }
            }
            warp_graphql::queries::get_user_settings::UserResult::Unknown => {
                Err(anyhow!("Unable to fetch user settings"))
            }
        }
    }

    // Returns a history of the current user's conversation usage over the past n days.
    async fn get_conversation_usage_history(
        &self,
        days: Option<i32>,
        limit: Option<i32>,
        last_updated_end_timestamp: Option<warp_graphql::scalars::Time>,
    ) -> Result<Vec<ConversationUsage>> {
        let operation = GetConversationUsage::build(GetConversationUsageVariables {
            request_context: get_request_context(),
            days,
            limit,
            last_updated_end_timestamp,
        });
        let response = self.send_graphql_request(operation, None).await?;
        match response.user {
            UserResult::UserOutput(out) => Ok(out.user.conversation_usage),
            UserResult::Unknown => Err(anyhow!("Unable to fetch conversation usage")),
        }
    }

    async fn set_is_telemetry_enabled(&self, value: bool) -> Result<()> {
        let variables = UpdateUserSettingsVariables {
            input: UpdateUserSettingsInput {
                telemetry_enabled: Some(value),
                ..Default::default()
            },
            request_context: get_request_context(),
        };

        let operation = UpdateUserSettings::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .update_user_settings;

        match result {
            UpdateUserSettingsResult::UpdateUserSettingsOutput(_) => Ok(()),
            UpdateUserSettingsResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            UpdateUserSettingsResult::Unknown => Err(anyhow!("failed to set telemetry enabled")),
        }
    }

    async fn set_is_crash_reporting_enabled(&self, value: bool) -> Result<()> {
        let variables = UpdateUserSettingsVariables {
            input: UpdateUserSettingsInput {
                crash_reporting_enabled: Some(value),
                ..Default::default()
            },
            request_context: get_request_context(),
        };

        let operation = UpdateUserSettings::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .update_user_settings;

        match result {
            UpdateUserSettingsResult::UpdateUserSettingsOutput(_) => Ok(()),
            UpdateUserSettingsResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            UpdateUserSettingsResult::Unknown => {
                Err(anyhow!("failed to set crash reporting enabled"))
            }
        }
    }

    async fn set_is_cloud_conversation_storage_enabled(&self, value: bool) -> Result<()> {
        let variables = UpdateUserSettingsVariables {
            input: UpdateUserSettingsInput {
                cloud_conversation_storage_enabled: Some(value),
                ..Default::default()
            },
            request_context: get_request_context(),
        };

        let operation = UpdateUserSettings::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .update_user_settings;

        match result {
            UpdateUserSettingsResult::UpdateUserSettingsOutput(_) => Ok(()),
            UpdateUserSettingsResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            UpdateUserSettingsResult::Unknown => {
                Err(anyhow!("failed to set cloud conversation storage enabled"))
            }
        }
    }

    async fn update_user_settings(&self, settings_snapshot: PrivacySettingsSnapshot) -> Result<()> {
        let variables = UpdateUserSettingsVariables {
            input: UpdateUserSettingsInput {
                telemetry_enabled: Some(settings_snapshot.is_telemetry_enabled()),
                crash_reporting_enabled: Some(settings_snapshot.is_crash_reporting_enabled()),
                cloud_conversation_storage_enabled: settings_snapshot
                    .cloud_conversation_storage_enabled(),
            },
            request_context: get_request_context(),
        };

        let operation = UpdateUserSettings::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .update_user_settings;

        match result {
            UpdateUserSettingsResult::UpdateUserSettingsOutput(_) => Ok(()),
            UpdateUserSettingsResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            UpdateUserSettingsResult::Unknown => Err(anyhow!("failed to update user settings")),
        }
    }

    async fn set_user_is_onboarded(&self) -> Result<bool> {
        let variables = SetUserIsOnboardedVariables {
            request_context: get_request_context(),
        };

        let operation = SetUserIsOnboarded::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .set_user_is_onboarded;

        match result {
            SetUserIsOnboardedResult::SetUserIsOnboardedOutput(_) => Ok(true),
            SetUserIsOnboardedResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            SetUserIsOnboardedResult::Unknown => Err(anyhow!("failed to set user is onboarded")),
        }
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

        // Firebase doesn't directly support the device flow. Instead, the server mints a short-lived
        // custom access token, which we can then exchange for a refresh token.
        Ok(FirebaseToken::Custom(
            result.access_token().secret().to_string(),
        ))
    }

    // API Keys
    async fn list_api_keys(&self) -> Result<Vec<ApiKeyProperties>> {
        let variables = ApiKeysVariables {
            request_context: get_request_context(),
        };
        let operation = ApiKeys::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.api_keys {
            ApiKeyPropertiesResult::ApiKeyPropertiesOutput(output) => Ok(output.api_keys),
            ApiKeyPropertiesResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            ApiKeyPropertiesResult::Unknown => Err(anyhow!("failed to fetch API keys")),
        }
    }

    async fn create_api_key(
        &self,
        name: String,
        team_id: Option<cynic::Id>,
        agent_uid: Option<cynic::Id>,
        expires_at: Option<warp_graphql::scalars::Time>,
    ) -> Result<GenerateApiKeyResult> {
        let variables = GenerateApiKeyVariables {
            input: GenerateApiKeyInput {
                name,
                team_id,
                agent_uid,
                expires_at,
            },
            request_context: get_request_context(),
        };
        let operation = GenerateApiKey::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        Ok(response.generate_api_key)
    }

    async fn list_agent_identities(&self) -> Result<Vec<AgentIdentity>> {
        let response: AgentIdentitiesResponse = self.get_public_api("agent/identities").await?;
        Ok(response.agents)
    }

    async fn expire_api_key(&self, key_uid: &ApiKeyUid) -> Result<ExpireApiKeyResult> {
        let variables = ExpireApiKeyVariables {
            key_uid: key_uid.into(),
            request_context: get_request_context(),
        };
        let op = ExpireApiKey::build(variables);
        let res = self.send_graphql_request(op, None).await?;
        Ok(res.expire_api_key)
    }

    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>> {
        if cfg!(target_family = "wasm") {
            return Ok(None);
        }

        // Check if we have a cached token that's still valid (with 5 minute buffer).
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
            Err(e) => return Err(e.into()),
        };

        let token_str = workload_token.token.clone();

        {
            let mut cached = self.ambient_workload_token.lock();
            *cached = Some(workload_token);
        }

        Ok(Some(token_str))
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
struct UserProperties {
    user: User,
    server_experiments: Vec<ServerExperiment>,
    llms: crate::ai::llms::ModelsByFeature,
    api_key_owner_type: Option<OwnerType>,
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
        let api_key_owner_type = user_output.api_key_owner_type;

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
            api_key_owner_type,
        }
    }
}

#[derive(Error, Debug)]
/// Error type when retrieving a user and validating it against Firebase.
pub enum UserAuthenticationError {
    /// The user's refresh token is invalid. This could occur if the user authed through
    /// e.g. Google/GitHub and changed their password.
    #[error("Firebase returned a token error when fetching an ID token")]
    DeniedAccessToken(FirebaseError),
    /// The user's account is invalid. This could occur if the user requested their account
    /// be deleted per their GDPR/CCPA rights.
    #[error("Firebase returned a user error when fetching an ID token")]
    UserAccountDisabled(FirebaseError),
    #[error("Invalid state parameter in auth redirect")]
    InvalidStateParameter,
    #[error("Missing state parameter in auth redirect")]
    MissingStateParameter,
    #[error("unexpected error occurred when fetching an ID token: {0:#}")]
    Unexpected(#[from] anyhow::Error),
}

impl ErrorExt for UserAuthenticationError {
    fn is_actionable(&self) -> bool {
        match self {
            UserAuthenticationError::DeniedAccessToken(err) => {
                // If a request to our server failed because the user's refresh token
                // has expired, they should re-auth, but there's no value in reporting
                // this back to us.
                log::info!("ignoring denied access token error: {err:#}");
                false
            }
            UserAuthenticationError::UserAccountDisabled(err) => {
                // Similarly, if their account is disabled, they can't make requests.
                log::info!("ignoring user account disabled error: {err:#}");
                false
            }
            UserAuthenticationError::Unexpected(err) => err.is_actionable(),
            UserAuthenticationError::InvalidStateParameter
            | UserAuthenticationError::MissingStateParameter => {
                // For now, we're marking these as actionable, since a surplus of these errors
                // could mean that something is wrong in our login flow (e.g. we're not properly
                // passing the `state` variable back to the desktop client).
                // But in general, someone attempting to trick another into logging into their
                // account with a spoofed `state` variable is not actionable.
                true
            }
        }
    }
}
register_error!(UserAuthenticationError);

impl From<FirebaseError> for UserAuthenticationError {
    fn from(error: FirebaseError) -> Self {
        if FETCH_ACCESS_TOKEN_SOFT_ERROR_MESSAGES.contains(&error.message.as_str()) {
            UserAuthenticationError::DeniedAccessToken(error)
        } else if FETCH_ACCESS_TOKEN_HARD_ERROR_MESSAGES.contains(&error.message.as_str()) {
            UserAuthenticationError::UserAccountDisabled(error)
        } else {
            UserAuthenticationError::Unexpected(
                anyhow::Error::from(error)
                    .context("Failed to exchange refresh token with access token."),
            )
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

#[derive(Error, Debug)]
/// Error type when minting a new custom token for an anonymous user
pub enum MintCustomTokenError {
    #[error("Received a user facing error: {0}")]
    UserFacingError(String),
    #[error("Failed to create new custom token with unknown error")]
    Unknown,
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
