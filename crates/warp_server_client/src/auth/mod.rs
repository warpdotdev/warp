mod session;

use std::result::Result as StdResult;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
use firebase::FirebaseError;
use instant::Duration;
#[cfg(any(test, feature = "test-util"))]
use mockall::automock;
pub use session::*;
use thiserror::Error;
pub use user_uid::{TEST_USER_EMAIL, TEST_USER_UID, UserUid};
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
pub use warp_server_auth::user_uid;

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
    /// The credentials used to authenticate this user.
    pub credentials: Credentials,
    /// Whether this attempt to fetch the user was for refreshing an existing logged-in user.
    pub from_refresh: bool,
}

#[cfg_attr(any(test, feature = "test-util"), automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait AuthClient: Send + Sync {
    /// Creates an anonymous user who is allowed to use Warp but may lack the ability
    /// to interact with particular features.
    async fn create_anonymous_user(
        &self,
        referral_code: Option<String>,
        anonymous_user_type: AnonymousUserType,
    ) -> Result<CreateAnonymousUserResult>;

    /// Returns the cached access token if it is still valid.
    ///
    /// If it has expired, this fetches a new access token using the user's refresh
    /// token, caches it, and then returns it. It may return an auth mode that does
    /// not require an Authorization header, such as session cookies or test credentials.
    async fn get_or_refresh_access_token(&self) -> Result<AuthToken>;

    /// Fetches the user's metadata and authentication tokens.
    async fn fetch_user(
        &self,
        token: LoginToken,
        for_refresh: bool,
    ) -> StdResult<FetchUserResult, UserAuthenticationError>;

    /// Creates and fetches a new custom token for the current user from Firebase.
    ///
    /// This only works for anonymous users and surfaces an error if the user is not anonymous.
    async fn fetch_new_custom_token(&self) -> Result<MintCustomTokenResult>;

    /// Handles the response from [`Self::fetch_new_custom_token`] by returning the newly minted custom token.
    fn on_custom_token_fetched(
        &self,
        response: Result<MintCustomTokenResult>,
    ) -> Result<String, MintCustomTokenError>;

    /// Queries warp-server for a set of the currently logged-in user's fields.
    async fn fetch_user_properties<'a>(&self, auth_token: Option<&'a str>)
    -> Result<GqlUserOutput>;

    /// Returns the user's settings retrieved from the server, if any.
    ///
    /// The user may not have server-side settings if they onboarded before telemetry
    /// opt-out launched, have not logged in since the launch, and have never changed
    /// defaults for any setting in [`SyncedUserSettings`]. If the fetched settings
    /// object exists but is missing required fields, or if the request itself fails,
    /// this returns an error.
    async fn get_user_settings(&self) -> Result<Option<SyncedUserSettings>>;

    async fn set_is_telemetry_enabled(&self, value: bool) -> Result<()>;

    async fn set_is_crash_reporting_enabled(&self, value: bool) -> Result<()>;

    async fn set_is_cloud_conversation_storage_enabled(&self, value: bool) -> Result<()>;

    /// Sends a request to update the user's settings on the server with values in the given input.
    async fn update_user_settings(&self, input: UpdateUserSettingsInput) -> Result<()>;

    async fn set_user_is_onboarded(&self) -> Result<bool>;

    /// Requests a device authorization code from the server for headless CLI or SDK authentication.
    async fn request_device_code(
        &self,
    ) -> StdResult<oauth2::StandardDeviceAuthorizationResponse, UserAuthenticationError>;

    /// Waits for the request to be approved or rejected and exchanges it for a short-lived custom access token.
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

    /// Fetches the list of named agent identities for the user's team.
    async fn list_agent_identities(&self) -> Result<Vec<AgentIdentity>>;

    /// Returns a cached ambient workload token, or issues a new one if none is present or it has expired.
    ///
    /// This returns `Ok(None)` if the process is not running in an isolation platform or on WASM.
    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>>;
}

/// Extracted auth API implementation over application-provided base capabilities.
pub struct AuthClientImpl {
    base_client: Arc<dyn BaseClient>,
    auth_session: Arc<AuthSession>,
}

impl AuthClientImpl {
    pub fn new(base_client: Arc<dyn BaseClient>, auth_session: Arc<AuthSession>) -> Self {
        Self {
            base_client,
            auth_session,
        }
    }

    async fn update_settings(
        &self,
        input: UpdateUserSettingsInput,
        unknown_error_message: &'static str,
    ) -> Result<()> {
        let operation = UpdateUserSettings::build(UpdateUserSettingsVariables {
            input,
            request_context: warp_graphql::client::get_request_context(),
        });
        let result = send_graphql_request(self.base_client.as_ref(), operation, None)
            .await?
            .update_user_settings;
        Self::on_settings_updated(result, unknown_error_message)
    }

    fn on_settings_updated(
        result: UpdateUserSettingsResult,
        unknown_error_message: &'static str,
    ) -> Result<()> {
        match result {
            UpdateUserSettingsResult::UpdateUserSettingsOutput(_) => Ok(()),
            UpdateUserSettingsResult::UserFacingError(error) => Err(anyhow!(
                warp_graphql::client::get_user_facing_error_message(error)
            )),
            UpdateUserSettingsResult::Unknown => Err(anyhow!(unknown_error_message)),
        }
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl AuthClient for AuthClientImpl {
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
        self.auth_session.get_or_refresh_access_token().await
    }

    async fn fetch_user(
        &self,
        token: LoginToken,
        for_refresh: bool,
    ) -> StdResult<FetchUserResult, UserAuthenticationError> {
        let new_credentials = self.auth_session.exchange_credentials(token).await?;
        let auth_token = new_credentials.bearer_token();
        let user_output = self
            .fetch_user_properties(auth_token.as_bearer_token())
            .await
            .context("Failed to fetch user response data")
            .map_err(UserAuthenticationError::Unexpected)?;
        // Store the owner type if using an API key.
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
        let response = send_graphql_request(self.base_client.as_ref(), operation, None).await?;
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
        let response = send_graphql_request(self.base_client.as_ref(), operation, None).await?;
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
        self.update_settings(
            UpdateUserSettingsInput {
                telemetry_enabled: Some(value),
                ..Default::default()
            },
            "failed to set telemetry enabled",
        )
        .await
    }

    async fn set_is_crash_reporting_enabled(&self, value: bool) -> Result<()> {
        self.update_settings(
            UpdateUserSettingsInput {
                crash_reporting_enabled: Some(value),
                ..Default::default()
            },
            "failed to set crash reporting enabled",
        )
        .await
    }

    async fn set_is_cloud_conversation_storage_enabled(&self, value: bool) -> Result<()> {
        self.update_settings(
            UpdateUserSettingsInput {
                cloud_conversation_storage_enabled: Some(value),
                ..Default::default()
            },
            "failed to set cloud conversation storage enabled",
        )
        .await
    }

    async fn update_user_settings(&self, input: UpdateUserSettingsInput) -> Result<()> {
        self.update_settings(input, "failed to update user settings")
            .await
    }

    async fn set_user_is_onboarded(&self) -> Result<bool> {
        let operation = SetUserIsOnboarded::build(SetUserIsOnboardedVariables {
            request_context: warp_graphql::client::get_request_context(),
        });
        let result = send_graphql_request(self.base_client.as_ref(), operation, None)
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
        self.auth_session.request_device_code().await
    }

    async fn exchange_device_access_token(
        &self,
        details: &oauth2::StandardDeviceAuthorizationResponse,
        timeout: Duration,
    ) -> StdResult<FirebaseToken, UserAuthenticationError> {
        self.auth_session
            .exchange_device_access_token(details, timeout)
            .await
    }

    async fn list_api_keys(&self) -> Result<Vec<ApiKeyProperties>> {
        let operation = ApiKeys::build(ApiKeysVariables {
            request_context: warp_graphql::client::get_request_context(),
        });
        let response = send_graphql_request(self.base_client.as_ref(), operation, None).await?;
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
        let response = send_graphql_request(self.base_client.as_ref(), operation, None).await?;
        Ok(response.generate_api_key)
    }

    async fn expire_api_key(&self, key_uid: &ApiKeyUid) -> Result<ExpireApiKeyResult> {
        let operation = ExpireApiKey::build(ExpireApiKeyVariables {
            key_uid: key_uid.into(),
            request_context: warp_graphql::client::get_request_context(),
        });
        let response = send_graphql_request(self.base_client.as_ref(), operation, None).await?;
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

/// Error type when retrieving a user and validating it against Firebase.
#[derive(Error, Debug)]
pub enum UserAuthenticationError {
    /// The user's refresh token is invalid, which can occur after the user changes
    /// a password for Google or GitHub authentication.
    #[error("Firebase returned a token error when fetching an ID token")]
    DeniedAccessToken(FirebaseError),
    /// The user's account is invalid, which can occur after the user requests
    /// account deletion under GDPR or CCPA.
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
                // If a request to our server failed because the user's refresh token
                // has expired, they should reauthenticate, but there is no value in
                // reporting this back to us.
                log::info!("ignoring denied access token error: {error:#}");
                false
            }
            UserAuthenticationError::UserAccountDisabled(error) => {
                // If the user's account is disabled, they cannot make requests.
                log::info!("ignoring user account disabled error: {error:#}");
                false
            }
            UserAuthenticationError::Unexpected(error) => error.is_actionable(),
            UserAuthenticationError::InvalidStateParameter
            | UserAuthenticationError::MissingStateParameter => {
                // These errors remain actionable because a surplus could indicate a problem in
                // the login flow, although an attempt to spoof the `state` variable is not actionable.
                true
            }
        }
    }
}
register_error!(UserAuthenticationError);

impl From<FirebaseError> for UserAuthenticationError {
    fn from(error: FirebaseError) -> Self {
        // These Firebase errors indicate that the user's token is in an errored state
        // and that the user likely just needs to log in again.
        const SOFT_ERRORS: &[&str] = &[
            "TOKEN_EXPIRED",
            "INVALID_REFRESH_TOKEN",
            "MISSING_REFRESH_TOKEN",
        ];
        // These Firebase errors indicate that the user's account is in an errored state
        // and that the user likely can no longer sign in with it.
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

/// Error type when minting a new custom token for an anonymous user.
#[derive(Error, Debug)]
pub enum MintCustomTokenError {
    #[error("Received a user facing error: {0}")]
    UserFacingError(String),
    #[error("Failed to create new custom token with unknown error")]
    Unknown,
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
