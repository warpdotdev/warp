use std::result::Result as StdResult;

use anyhow::{Context as _, Result, anyhow};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
use firebase::FirebaseError;
use instant::Duration;
#[cfg(any(test, feature = "test-util"))]
use mockall::automock;
use thiserror::Error;
use warp_core::errors::{AnyhowErrorExt, ErrorExt, register_error};
use warp_graphql::client::Operation;
use warp_graphql::mutations::create_anonymous_user::{
    AnonymousUserType, CreateAnonymousUser, CreateAnonymousUserResult, CreateAnonymousUserVariables,
};
use warp_graphql::mutations::expire_api_key::{
    ExpireApiKey, ExpireApiKeyResult, ExpireApiKeyVariables,
};
use warp_graphql::mutations::generate_api_key::{
    GenerateApiKey, GenerateApiKeyInput, GenerateApiKeyResult, GenerateApiKeyVariables,
};
use warp_graphql::mutations::mint_custom_token::{MintCustomTokenResult, MintCustomTokenVariables};
use warp_graphql::mutations::set_user_is_onboarded::{
    SetUserIsOnboarded, SetUserIsOnboardedResult, SetUserIsOnboardedVariables,
};
use warp_graphql::mutations::update_user_settings::{
    UpdateUserSettings, UpdateUserSettingsInput, UpdateUserSettingsResult,
    UpdateUserSettingsVariables,
};
use warp_graphql::queries::api_keys::{
    ApiKeyProperties, ApiKeyPropertiesResult, ApiKeys, ApiKeysVariables,
};
use warp_graphql::queries::get_user::{GetUser, GetUserVariables, UserOutput as GqlUserOutput};
use warp_graphql::queries::get_user_settings::{GetUserSettings, GetUserSettingsVariables};
use warp_server_auth::credentials::{AuthToken, Credentials, FirebaseToken, LoginToken};

use crate::base_client::BaseClient;
use crate::graphql_helpers::send_graphql_request;
use crate::ids::ApiKeyUid;

/// Header key used to associate unauthenticated requests with an experiment identity.
pub const EXPERIMENT_ID_HEADER: &str = "X-Warp-Experiment-Id";

/// A named agent identity from the public API.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AgentIdentity {
    pub uid: String,
    pub name: String,
    pub available: bool,
}

/// User settings that are stored server-side on a per-user basis.
#[derive(Copy, Clone, Debug, Default)]
pub struct SyncedUserSettings {
    pub is_cloud_conversation_storage_enabled: bool,
    pub is_crash_reporting_enabled: bool,
    pub is_telemetry_enabled: bool,
}

/// Protocol-level results of fetching the current user.
pub struct FetchUserResult {
    pub user_output: GqlUserOutput,
    pub credentials: Credentials,
    /// Whether this attempt to fetch the user was for refreshing an existing logged-in user.
    pub from_refresh: bool,
}

#[cfg_attr(any(test, feature = "test-util"), automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait AuthClient: Send + Sync {
    async fn create_anonymous_user(
        &self,
        referral_code: Option<String>,
        anonymous_user_type: AnonymousUserType,
    ) -> Result<CreateAnonymousUserResult>;

    async fn get_or_refresh_access_token(&self) -> Result<AuthToken>;

    async fn fetch_user(
        &self,
        token: LoginToken,
        for_refresh: bool,
    ) -> StdResult<FetchUserResult, UserAuthenticationError>;

    async fn fetch_new_custom_token(&self) -> Result<MintCustomTokenResult>;

    fn on_custom_token_fetched(
        &self,
        response: Result<MintCustomTokenResult>,
    ) -> Result<String, MintCustomTokenError>;

    async fn fetch_user_properties<'a>(&self, auth_token: Option<&'a str>)
    -> Result<GqlUserOutput>;

    async fn get_user_settings(&self) -> Result<Option<SyncedUserSettings>>;

    async fn set_is_telemetry_enabled(&self, value: bool) -> Result<()>;

    async fn set_is_crash_reporting_enabled(&self, value: bool) -> Result<()>;

    async fn set_is_cloud_conversation_storage_enabled(&self, value: bool) -> Result<()>;

    async fn update_user_settings(&self, input: UpdateUserSettingsInput) -> Result<()>;

    async fn set_user_is_onboarded(&self) -> Result<bool>;

    async fn request_device_code(
        &self,
    ) -> StdResult<oauth2::StandardDeviceAuthorizationResponse, UserAuthenticationError>;

    async fn exchange_device_access_token(
        &self,
        details: &oauth2::StandardDeviceAuthorizationResponse,
        timeout: Duration,
    ) -> StdResult<FirebaseToken, UserAuthenticationError>;

    async fn list_api_keys(&self) -> Result<Vec<ApiKeyProperties>>;

    async fn create_api_key(
        &self,
        name: String,
        team_id: Option<cynic::Id>,
        agent_uid: Option<cynic::Id>,
        expires_at: Option<warp_graphql::scalars::Time>,
    ) -> Result<GenerateApiKeyResult>;

    async fn expire_api_key(&self, key_uid: &ApiKeyUid) -> Result<ExpireApiKeyResult>;

    async fn list_agent_identities(&self) -> Result<Vec<AgentIdentity>>;

    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>>;
}

/// Extracted auth API implementation over application-provided base capabilities.
pub struct AuthClientImpl<'a> {
    base_client: &'a dyn BaseClient,
}

impl<'a> AuthClientImpl<'a> {
    pub fn new(base_client: &'a dyn BaseClient) -> Self {
        Self { base_client }
    }

    async fn update_settings(&self, input: UpdateUserSettingsInput) -> Result<()> {
        let operation = UpdateUserSettings::build(UpdateUserSettingsVariables {
            input,
            request_context: warp_graphql::client::get_request_context(),
        });
        let result = send_graphql_request(self.base_client, operation, None)
            .await?
            .update_user_settings;
        match result {
            UpdateUserSettingsResult::UpdateUserSettingsOutput(_) => Ok(()),
            UpdateUserSettingsResult::UserFacingError(error) => Err(anyhow!(
                warp_graphql::client::get_user_facing_error_message(error)
            )),
            UpdateUserSettingsResult::Unknown => Err(anyhow!("failed to update user settings")),
        }
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl AuthClient for AuthClientImpl<'_> {
    async fn create_anonymous_user(
        &self,
        referral_code: Option<String>,
        anonymous_user_type: AnonymousUserType,
    ) -> Result<CreateAnonymousUserResult> {
        let operation = CreateAnonymousUser::build(CreateAnonymousUserVariables {
            input: warp_graphql::mutations::create_anonymous_user::CreateAnonymousUserInput {
                anonymous_user_type,
                expiration_type: warp_graphql::mutations::create_anonymous_user::AnonymousUserExpirationType::NoExpiration,
                referral_code,
            },
            request_context: warp_graphql::client::get_request_context(),
        });
        let response = operation
            .send_request(
                self.base_client.http_client(),
                self.base_client.unauthenticated_graphql_request_options(),
            )
            .await?;
        Ok(response
            .data
            .ok_or_else(|| anyhow!("missing data in response"))?
            .create_anonymous_user)
    }

    async fn get_or_refresh_access_token(&self) -> Result<AuthToken> {
        self.base_client.get_or_refresh_access_token().await
    }

    async fn fetch_user(
        &self,
        token: LoginToken,
        for_refresh: bool,
    ) -> StdResult<FetchUserResult, UserAuthenticationError> {
        let new_credentials = self.base_client.exchange_credentials(token).await?;
        let auth_token = new_credentials.bearer_token();
        let user_output = self
            .fetch_user_properties(auth_token.as_bearer_token())
            .await
            .context("Failed to fetch user response data")
            .map_err(UserAuthenticationError::Unexpected)?;
        let new_credentials = match new_credentials {
            Credentials::ApiKey { key, .. } => Credentials::ApiKey {
                key,
                owner_type: user_output.api_key_owner_type,
            },
            other => other,
        };
        Ok(FetchUserResult {
            user_output,
            credentials: new_credentials,
            from_refresh: for_refresh,
        })
    }

    async fn fetch_new_custom_token(&self) -> Result<MintCustomTokenResult> {
        let operation = warp_graphql::mutations::mint_custom_token::MintCustomToken::build(
            MintCustomTokenVariables {
                request_context: warp_graphql::client::get_request_context(),
            },
        );
        let response = send_graphql_request(self.base_client, operation, None).await?;
        Ok(response.mint_custom_token)
    }

    fn on_custom_token_fetched(
        &self,
        response: Result<MintCustomTokenResult>,
    ) -> Result<String, MintCustomTokenError> {
        match response {
            Ok(MintCustomTokenResult::MintCustomTokenOutput(output)) => Ok(output.custom_token),
            Ok(MintCustomTokenResult::UserFacingError(error)) => {
                Err(MintCustomTokenError::UserFacingError(
                    warp_graphql::client::get_user_facing_error_message(error),
                ))
            }
            Ok(MintCustomTokenResult::Unknown) | Err(_) => Err(MintCustomTokenError::Unknown),
        }
    }

    async fn fetch_user_properties<'a>(
        &self,
        auth_token: Option<&'a str>,
    ) -> Result<GqlUserOutput> {
        let operation = GetUser::build(GetUserVariables {
            request_context: warp_graphql::client::get_request_context(),
        });
        let mut options = self.base_client.unauthenticated_graphql_request_options();
        options.auth_token = auth_token.map(ToOwned::to_owned);
        options.headers.insert(
            EXPERIMENT_ID_HEADER.to_string(),
            self.base_client.anonymous_id(),
        );
        let response = operation
            .send_request(self.base_client.http_client(), options)
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
        let operation = GetUserSettings::build(GetUserSettingsVariables {
            request_context: warp_graphql::client::get_request_context(),
        });
        let response = send_graphql_request(self.base_client, operation, None).await?;
        match response.user {
            warp_graphql::queries::get_user_settings::UserResult::UserOutput(user_output) => {
                Ok(user_output
                    .user
                    .settings
                    .map(|settings| SyncedUserSettings {
                        is_cloud_conversation_storage_enabled: settings
                            .is_cloud_conversation_storage_enabled,
                        is_crash_reporting_enabled: settings.is_crash_reporting_enabled,
                        is_telemetry_enabled: settings.is_telemetry_enabled,
                    }))
            }
            warp_graphql::queries::get_user_settings::UserResult::Unknown => {
                Err(anyhow!("Unable to fetch user settings"))
            }
        }
    }

    async fn set_is_telemetry_enabled(&self, value: bool) -> Result<()> {
        self.update_settings(UpdateUserSettingsInput {
            telemetry_enabled: Some(value),
            ..Default::default()
        })
        .await
    }

    async fn set_is_crash_reporting_enabled(&self, value: bool) -> Result<()> {
        self.update_settings(UpdateUserSettingsInput {
            crash_reporting_enabled: Some(value),
            ..Default::default()
        })
        .await
    }

    async fn set_is_cloud_conversation_storage_enabled(&self, value: bool) -> Result<()> {
        self.update_settings(UpdateUserSettingsInput {
            cloud_conversation_storage_enabled: Some(value),
            ..Default::default()
        })
        .await
    }

    async fn update_user_settings(&self, input: UpdateUserSettingsInput) -> Result<()> {
        self.update_settings(input).await
    }

    async fn set_user_is_onboarded(&self) -> Result<bool> {
        let operation = SetUserIsOnboarded::build(SetUserIsOnboardedVariables {
            request_context: warp_graphql::client::get_request_context(),
        });
        let result = send_graphql_request(self.base_client, operation, None)
            .await?
            .set_user_is_onboarded;
        match result {
            SetUserIsOnboardedResult::SetUserIsOnboardedOutput(_) => Ok(true),
            SetUserIsOnboardedResult::UserFacingError(error) => Err(anyhow!(
                warp_graphql::client::get_user_facing_error_message(error)
            )),
            SetUserIsOnboardedResult::Unknown => Err(anyhow!("failed to set user is onboarded")),
        }
    }

    async fn request_device_code(
        &self,
    ) -> StdResult<oauth2::StandardDeviceAuthorizationResponse, UserAuthenticationError> {
        self.base_client.request_device_code().await
    }

    async fn exchange_device_access_token(
        &self,
        details: &oauth2::StandardDeviceAuthorizationResponse,
        timeout: Duration,
    ) -> StdResult<FirebaseToken, UserAuthenticationError> {
        self.base_client
            .exchange_device_access_token(details, timeout)
            .await
    }

    async fn list_api_keys(&self) -> Result<Vec<ApiKeyProperties>> {
        let operation = ApiKeys::build(ApiKeysVariables {
            request_context: warp_graphql::client::get_request_context(),
        });
        let response = send_graphql_request(self.base_client, operation, None).await?;
        match response.api_keys {
            ApiKeyPropertiesResult::ApiKeyPropertiesOutput(output) => Ok(output.api_keys),
            ApiKeyPropertiesResult::UserFacingError(error) => Err(anyhow!(
                warp_graphql::client::get_user_facing_error_message(error)
            )),
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
        let operation = GenerateApiKey::build(GenerateApiKeyVariables {
            input: GenerateApiKeyInput {
                name,
                team_id,
                agent_uid,
                expires_at,
            },
            request_context: warp_graphql::client::get_request_context(),
        });
        let response = send_graphql_request(self.base_client, operation, None).await?;
        Ok(response.generate_api_key)
    }

    async fn expire_api_key(&self, key_uid: &ApiKeyUid) -> Result<ExpireApiKeyResult> {
        let operation = ExpireApiKey::build(ExpireApiKeyVariables {
            key_uid: key_uid.into(),
            request_context: warp_graphql::client::get_request_context(),
        });
        let response = send_graphql_request(self.base_client, operation, None).await?;
        Ok(response.expire_api_key)
    }

    async fn list_agent_identities(&self) -> Result<Vec<AgentIdentity>> {
        self.base_client.list_agent_identities().await
    }

    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>> {
        self.base_client
            .get_or_create_ambient_workload_token()
            .await
    }
}

#[derive(Error, Debug)]
pub enum UserAuthenticationError {
    #[error("Firebase returned a token error when fetching an ID token")]
    DeniedAccessToken(FirebaseError),
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
            UserAuthenticationError::DeniedAccessToken(error) => {
                log::info!("ignoring denied access token error: {error:#}");
                false
            }
            UserAuthenticationError::UserAccountDisabled(error) => {
                log::info!("ignoring user account disabled error: {error:#}");
                false
            }
            UserAuthenticationError::Unexpected(error) => error.is_actionable(),
            UserAuthenticationError::InvalidStateParameter
            | UserAuthenticationError::MissingStateParameter => true,
        }
    }
}
register_error!(UserAuthenticationError);

impl From<FirebaseError> for UserAuthenticationError {
    fn from(error: FirebaseError) -> Self {
        const SOFT_ERRORS: &[&str] = &[
            "TOKEN_EXPIRED",
            "INVALID_REFRESH_TOKEN",
            "MISSING_REFRESH_TOKEN",
        ];
        const HARD_ERRORS: &[&str] = &["USER_DISABLED", "USER_NOT_FOUND"];
        if SOFT_ERRORS.contains(&error.message.as_str()) {
            UserAuthenticationError::DeniedAccessToken(error)
        } else if HARD_ERRORS.contains(&error.message.as_str()) {
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
pub enum MintCustomTokenError {
    #[error("Received a user facing error: {0}")]
    UserFacingError(String),
    #[error("Failed to create new custom token with unknown error")]
    Unknown,
}
