use std::result::Result as StdResult;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use instant::Duration;

use super::session::AUTH_UNAVAILABLE;
use super::{
    AgentIdentity, AnonymousUserType, ApiKeyProperties, ApiKeyUid, AuthClient, AuthToken,
    CreateAnonymousUserResult, ExpireApiKeyResult, FetchUserResult, FirebaseToken,
    GenerateApiKeyResult, GqlUserOutput, LoginToken, MintCustomTokenError, MintCustomTokenResult,
    ServerId, SyncedUserSettings, UpdateUserSettingsInput, UserAuthenticationError,
};

pub struct AuthClientImpl;

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl AuthClient for AuthClientImpl {
    async fn create_anonymous_user(
        &self,
        _: Option<String>,
        _: AnonymousUserType,
    ) -> Result<CreateAnonymousUserResult> {
        bail!(AUTH_UNAVAILABLE)
    }

    async fn get_or_refresh_access_token(&self) -> Result<AuthToken> {
        bail!(AUTH_UNAVAILABLE)
    }

    async fn fetch_user(
        &self,
        _: LoginToken,
        _: bool,
    ) -> StdResult<FetchUserResult, UserAuthenticationError> {
        Err(anyhow!(AUTH_UNAVAILABLE).into())
    }

    async fn fetch_new_custom_token(&self) -> Result<MintCustomTokenResult> {
        bail!(AUTH_UNAVAILABLE)
    }

    fn on_custom_token_fetched(
        &self,
        _: Result<MintCustomTokenResult>,
    ) -> Result<String, MintCustomTokenError> {
        Err(MintCustomTokenError::UserFacingError(
            AUTH_UNAVAILABLE.to_owned(),
        ))
    }

    async fn fetch_user_properties<'a>(&self, _: Option<&'a str>) -> Result<GqlUserOutput> {
        bail!(AUTH_UNAVAILABLE)
    }

    async fn get_user_settings(&self) -> Result<Option<SyncedUserSettings>> {
        bail!(AUTH_UNAVAILABLE)
    }

    async fn set_is_telemetry_enabled(&self, _: bool) -> Result<()> {
        bail!(AUTH_UNAVAILABLE)
    }

    async fn set_is_crash_reporting_enabled(&self, _: bool) -> Result<()> {
        bail!(AUTH_UNAVAILABLE)
    }

    async fn set_is_cloud_conversation_storage_enabled(&self, _: bool) -> Result<()> {
        bail!(AUTH_UNAVAILABLE)
    }

    async fn update_user_settings(&self, _: UpdateUserSettingsInput) -> Result<()> {
        bail!(AUTH_UNAVAILABLE)
    }

    async fn set_user_is_onboarded(&self) -> Result<bool> {
        bail!(AUTH_UNAVAILABLE)
    }

    async fn request_device_code(
        &self,
    ) -> StdResult<oauth2::StandardDeviceAuthorizationResponse, UserAuthenticationError> {
        Err(anyhow!(AUTH_UNAVAILABLE).into())
    }

    async fn exchange_device_access_token(
        &self,
        _: &oauth2::StandardDeviceAuthorizationResponse,
        _: Duration,
    ) -> StdResult<FirebaseToken, UserAuthenticationError> {
        Err(anyhow!(AUTH_UNAVAILABLE).into())
    }

    async fn list_api_keys(&self, _: Option<ServerId>) -> Result<Vec<ApiKeyProperties>> {
        bail!(AUTH_UNAVAILABLE)
    }

    async fn create_api_key(
        &self,
        _: String,
        _: Option<cynic::Id>,
        _: Option<cynic::Id>,
        _: Option<warp_graphql::scalars::Time>,
    ) -> Result<GenerateApiKeyResult> {
        bail!(AUTH_UNAVAILABLE)
    }

    async fn expire_api_key(&self, _: &ApiKeyUid) -> Result<ExpireApiKeyResult> {
        bail!(AUTH_UNAVAILABLE)
    }

    async fn list_agent_identities(&self, _: Option<ServerId>) -> Result<Vec<AgentIdentity>> {
        bail!(AUTH_UNAVAILABLE)
    }
}

#[cfg(test)]
#[path = "local_auth_client_tests.rs"]
mod tests;
