use std::fmt;
use std::result::Result as StdResult;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use firebase::FetchAccessTokenResponse;
use instant::Duration;
use oauth2::TokenResponse as _;
use url::Url;
use warp_core::channel::ChannelState;
use warp_server_auth::auth_state::AuthState;
use warp_server_auth::credentials::{
    AuthToken, Credentials, FirebaseToken, LoginToken, RefreshToken,
};
use warp_server_auth::user::FirebaseAuthTokens;
use warpui_core::r#async::{BoxFuture, Timer};

use super::UserAuthenticationError;

const FETCH_ACCESS_TOKEN_TIMEOUT: Duration = Duration::from_secs(5);

/// Reports authentication or authenticated-access lifecycle changes to the application.
#[derive(Clone)]
pub enum AuthEvent {
    /// A staging API call was blocked, which may indicate a firewall misconfiguration.
    StagingAccessBlocked,
    /// The user's access token was invalid, so they need to reauthenticate.
    NeedsReauth,
    /// The user's account has been disabled.
    UserAccountDisabled,
    /// The current bearer token was refreshed.
    AccessTokenRefreshed {
        #[cfg_attr(target_family = "wasm", allow(dead_code))]
        token: String,
    },
    /// An Identity-Aware Proxy challenge was received.
    IapChallengeReceived,
}

impl fmt::Debug for AuthEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StagingAccessBlocked => f.write_str("StagingAccessBlocked"),
            Self::NeedsReauth => f.write_str("NeedsReauth"),
            Self::UserAccountDisabled => f.write_str("UserAccountDisabled"),
            Self::AccessTokenRefreshed { .. } => f
                .debug_struct("AccessTokenRefreshed")
                .field("token", &"<redacted>")
                .finish(),
            Self::IapChallengeReceived => f.write_str("IapChallengeReceived"),
        }
    }
}

/// The OAuth client type configured for Warp's device authorization endpoints.
type OAuth2Client = oauth2::basic::BasicClient<
    oauth2::EndpointNotSet,
    oauth2::EndpointSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointSet,
>;

/// Shared authentication-session mechanics used by extracted and app-owned API clients.
pub struct AuthSession {
    client: Arc<http_client::Client>,
    auth_state: Arc<AuthState>,
    event_sender: async_channel::Sender<AuthEvent>,
    oauth_client: OAuth2Client,
}

impl AuthSession {
    pub fn new(
        client: Arc<http_client::Client>,
        auth_state: Arc<AuthState>,
        event_sender: async_channel::Sender<AuthEvent>,
    ) -> Self {
        Self {
            client,
            auth_state,
            event_sender,
            oauth_client: Self::create_oauth_client(),
        }
    }

    pub fn allowed_to_refresh_token(&self) -> bool {
        self.auth_state
            .credentials()
            .is_none_or(|credentials| !credentials.is_externally_managed())
    }

    pub async fn get_or_refresh_access_token(&self) -> Result<AuthToken> {
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
                    let result = self.fetch_auth_tokens(firebase_token).await;

                    if let Err(UserAuthenticationError::DeniedAccessToken(_)) = result {
                        let _ = self.event_sender.send(AuthEvent::NeedsReauth).await;
                    }
                    let new_firebase_token_info = result?;
                    self.auth_state
                        .update_firebase_tokens(new_firebase_token_info.clone());
                    let _ = self
                        .event_sender
                        .send(AuthEvent::AccessTokenRefreshed {
                            token: new_firebase_token_info.id_token.clone(),
                        })
                        .await;
                    Ok(AuthToken::Firebase(new_firebase_token_info.id_token))
                } else {
                    Ok(AuthToken::Firebase(auth_tokens.id_token))
                }
            }
            Credentials::SessionCookie => Ok(AuthToken::NoAuth),
            #[cfg(any(feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => Ok(AuthToken::NoAuth),
        }
    }

    pub async fn exchange_credentials(
        &self,
        token: LoginToken,
    ) -> StdResult<Credentials, UserAuthenticationError> {
        match token {
            LoginToken::Firebase(firebase_token) => {
                let tokens = self.fetch_auth_tokens(firebase_token).await?;
                Ok(Credentials::Firebase(tokens))
            }
            LoginToken::ApiKey(key) => Ok(Credentials::ApiKey {
                key,
                owner_type: None,
            }),
            LoginToken::SessionCookie => Ok(Credentials::SessionCookie),
        }
    }

    pub async fn request_device_code(
        &self,
    ) -> StdResult<oauth2::StandardDeviceAuthorizationResponse, UserAuthenticationError> {
        self.oauth_client
            .exchange_device_code()
            .request_async(self.client.as_ref())
            .await
            .context("Failed to generate device code")
            .map_err(UserAuthenticationError::Unexpected)
    }

    pub async fn exchange_device_access_token(
        &self,
        details: &oauth2::StandardDeviceAuthorizationResponse,
        timeout: Duration,
    ) -> StdResult<FirebaseToken, UserAuthenticationError> {
        let result = self
            .oauth_client
            .exchange_device_access_token(details)
            .request_async(
                self.client.as_ref(),
                |delay| async move {
                    let _ = Timer::after(delay).await;
                },
                Some(timeout),
            )
            .await
            .context("Unable to obtain access token")
            .map_err(UserAuthenticationError::Unexpected)?;
        Ok(FirebaseToken::Custom(
            result.access_token().secret().to_string(),
        ))
    }

    fn create_oauth_client() -> OAuth2Client {
        let server_root =
            Url::parse(&ChannelState::server_root_url()).expect("Server root URL must be valid");
        let token_url = server_root
            .join("/api/v1/oauth/token")
            .expect("Invalid token URL");
        let device_url = server_root
            .join("/api/v1/oauth/device/auth")
            .expect("Invalid device URL");

        oauth2::basic::BasicClient::new(oauth2::ClientId::new("warp-cli".to_string()))
            .set_token_uri(oauth2::TokenUrl::from_url(token_url))
            .set_device_authorization_url(oauth2::DeviceAuthorizationUrl::from_url(device_url))
    }

    fn fetch_auth_tokens(
        &self,
        token: FirebaseToken,
    ) -> BoxFuture<'static, StdResult<FirebaseAuthTokens, UserAuthenticationError>> {
        let client = self.client.clone();
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

                        Self::fetch_access_token_via_proxy(client, &request_body, proxy_url).await
                    }
                },
                Err(error) => {
                    log::warn!(
                        "Failed to make response to firebase to fetch access token: {error:?}"
                    );

                    Self::fetch_access_token_via_proxy(client, &request_body, proxy_url).await
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
}
