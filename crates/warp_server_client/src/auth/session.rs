use std::fmt;
use std::result::Result as StdResult;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use firebase::{FetchAccessTokenResponse, FirebaseError};
use futures::lock::Mutex as AsyncMutex;
use http::StatusCode;
use http::header::RETRY_AFTER;
use instant::{Duration, Instant};
use oauth2::TokenResponse as _;
use parking_lot::Mutex;
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
const INITIAL_PROXY_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_PROXY_RETRY_DELAY: Duration = Duration::from_secs(5 * 60);

fn parse_retry_after(value: &str, now: chrono::DateTime<chrono::Utc>) -> Option<Duration> {
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let retry_at = chrono::DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&chrono::Utc);
    retry_at.signed_duration_since(now).to_std().ok()
}

#[derive(Debug)]
enum TokenRefreshError {
    Firebase(FirebaseError),
    ProxyTransient { retry_after: Option<Duration> },
    Unexpected(anyhow::Error),
}

impl TokenRefreshError {
    fn into_user_authentication_error(self) -> UserAuthenticationError {
        match self {
            Self::Firebase(error) => error.into(),
            Self::ProxyTransient { .. } => UserAuthenticationError::Unexpected(anyhow::anyhow!(
                "Firebase token proxy is temporarily unavailable"
            )),
            Self::Unexpected(error) => UserAuthenticationError::Unexpected(error),
        }
    }
}

#[derive(Default)]
struct RefreshState {
    refresh_token: Option<String>,
    terminal_error: Option<FirebaseError>,
    retry_at: Option<Instant>,
    consecutive_transient_failures: u32,
}

impl RefreshState {
    fn update_credentials(&mut self, refresh_token: &str) {
        if self.refresh_token.as_deref() != Some(refresh_token) {
            self.refresh_token = Some(refresh_token.to_string());
            self.clear_failure();
        }
    }

    fn clear_failure(&mut self) {
        self.terminal_error = None;
        self.retry_at = None;
        self.consecutive_transient_failures = 0;
    }

    fn record_terminal_failure(&mut self, error: FirebaseError) {
        self.terminal_error = Some(error);
        self.retry_at = None;
    }

    fn record_transient_failure(&mut self, now: Instant, retry_after: Option<Duration>) {
        self.consecutive_transient_failures = self.consecutive_transient_failures.saturating_add(1);
        let delay = retry_after.unwrap_or_else(|| {
            let exponent = self.consecutive_transient_failures.saturating_sub(1).min(4);
            INITIAL_PROXY_RETRY_DELAY
                .checked_mul(1 << exponent)
                .unwrap_or(MAX_PROXY_RETRY_DELAY)
        });
        let delay = delay.clamp(Duration::from_secs(1), MAX_PROXY_RETRY_DELAY);
        self.retry_at = now.checked_add(delay);
    }
}

/// Authentication and authenticated-transport conditions observed by shared client code.
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

/// Reusable authentication-session mechanics for server clients.
///
/// An `AuthSession` combines authentication state with the HTTP transport required to
/// exchange credentials, refresh access tokens, and complete OAuth device authorization.
/// Changes in authentication state that may require reactions from application logic are
/// emitted through an [`AuthEvent`] channel.
pub struct AuthSession {
    client: Arc<http_client::Client>,
    auth_state: Arc<AuthState>,
    event_sender: async_channel::Sender<AuthEvent>,
    oauth_client: OAuth2Client,
    refresh_lock: AsyncMutex<()>,
    refresh_state: Mutex<RefreshState>,
    #[cfg(test)]
    refresh_urls: Option<(String, String)>,
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
            refresh_lock: AsyncMutex::new(()),
            refresh_state: Mutex::new(RefreshState::default()),
            #[cfg(test)]
            refresh_urls: None,
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
                if !Self::firebase_tokens_need_refresh(&auth_tokens) {
                    Ok(AuthToken::Firebase(auth_tokens.id_token))
                } else {
                    self.refresh_firebase_access_token().await
                }
            }
            Credentials::SessionCookie => Ok(AuthToken::NoAuth),
            #[cfg(any(feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => Ok(AuthToken::NoAuth),
        }
    }

    fn firebase_tokens_need_refresh(auth_tokens: &FirebaseAuthTokens) -> bool {
        // Generate a new ID token if the token has expired or will expire in the
        // next five minutes. This matches the behavior of the Firebase Auth SDK.
        chrono::Local::now().fixed_offset() + chrono::Duration::minutes(5)
            >= auth_tokens.expiration_time
    }

    async fn refresh_firebase_access_token(&self) -> Result<AuthToken> {
        let _refresh_guard = self.refresh_lock.lock().await;
        let Some(Credentials::Firebase(auth_tokens)) = self.auth_state.credentials() else {
            bail!("Firebase credentials changed while refreshing the access token");
        };
        if !Self::firebase_tokens_need_refresh(&auth_tokens) {
            return Ok(AuthToken::Firebase(auth_tokens.id_token));
        }

        let refresh_token = auth_tokens.refresh_token;
        {
            let mut state = self.refresh_state.lock();
            state.update_credentials(&refresh_token);
            if let Some(error) = state.terminal_error.clone() {
                return Err(UserAuthenticationError::from(error).into());
            }
            if state
                .retry_at
                .is_some_and(|retry_at| Instant::now() < retry_at)
            {
                bail!("Firebase token refresh is temporarily unavailable");
            }
        }

        let firebase_token = FirebaseToken::Refresh(RefreshToken::new(refresh_token));
        match self.fetch_auth_tokens(firebase_token).await {
            Ok(new_auth_tokens) => {
                self.refresh_state.lock().clear_failure();
                self.auth_state
                    .update_firebase_tokens(new_auth_tokens.clone());
                let _ = self
                    .event_sender
                    .send(AuthEvent::AccessTokenRefreshed {
                        token: new_auth_tokens.id_token.clone(),
                    })
                    .await;
                Ok(AuthToken::Firebase(new_auth_tokens.id_token))
            }
            Err(error) => {
                let mut needs_reauth = false;
                {
                    let mut state = self.refresh_state.lock();
                    match &error {
                        TokenRefreshError::Firebase(firebase_error)
                            if matches!(
                                UserAuthenticationError::from(firebase_error.clone()),
                                UserAuthenticationError::DeniedAccessToken(_)
                            ) =>
                        {
                            state.record_terminal_failure(firebase_error.clone());
                            needs_reauth = true;
                        }
                        TokenRefreshError::ProxyTransient { retry_after } => {
                            state.record_transient_failure(Instant::now(), *retry_after);
                        }
                        TokenRefreshError::Firebase(_) | TokenRefreshError::Unexpected(_) => {}
                    }
                }
                if needs_reauth {
                    let _ = self.event_sender.send(AuthEvent::NeedsReauth).await;
                }
                Err(error.into_user_authentication_error().into())
            }
        }
    }

    /// Exchanges a long-lived token for fresh [`Credentials`].
    pub async fn exchange_credentials(
        &self,
        token: LoginToken,
    ) -> StdResult<Credentials, UserAuthenticationError> {
        match token {
            LoginToken::Firebase(firebase_token) => {
                let tokens = self
                    .fetch_auth_tokens(firebase_token)
                    .await
                    .map_err(TokenRefreshError::into_user_authentication_error)?;
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
        // Firebase does not directly support the device flow, so the server mints a
        // short-lived custom access token that can be exchanged for a refresh token.
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

        oauth2::basic::BasicClient::new(oauth2::ClientId::new("warp-agent-cli".to_string()))
            .set_token_uri(oauth2::TokenUrl::from_url(token_url))
            .set_device_authorization_url(oauth2::DeviceAuthorizationUrl::from_url(device_url))
    }

    fn fetch_auth_tokens(
        &self,
        token: FirebaseToken,
    ) -> BoxFuture<'static, StdResult<FirebaseAuthTokens, TokenRefreshError>> {
        let client = self.client.clone();
        let firebase_api_key = ChannelState::firebase_api_key();
        let direct_url = token.access_token_url(&firebase_api_key);
        let proxy_url = token.proxy_url(&ChannelState::server_root_url(), &firebase_api_key);
        #[cfg(test)]
        let (direct_url, proxy_url) = self.refresh_urls.clone().unwrap_or((direct_url, proxy_url));
        Box::pin(async move {
            let request_body = token.access_token_request_body();
            let (response, used_proxy) = match client
                .post(&direct_url)
                .form(&request_body)
                .timeout(FETCH_ACCESS_TOKEN_TIMEOUT)
                .send()
                .await
            {
                Ok(response) => (response, false),
                Err(_) => {
                    log::warn!("Firebase token request failed to send; retrying through proxy");
                    (
                        Self::fetch_access_token_via_proxy(client, &request_body, proxy_url)
                            .await?,
                        true,
                    )
                }
            };

            let response = response
                .json::<FetchAccessTokenResponse>()
                .await
                .map_err(|error| {
                    if used_proxy {
                        TokenRefreshError::ProxyTransient { retry_after: None }
                    } else {
                        TokenRefreshError::Unexpected(error.into())
                    }
                })?;
            match response {
                FetchAccessTokenResponse::Success {
                    id_token,
                    expires_in,
                    refresh_token,
                } => FirebaseAuthTokens::from_response(id_token, refresh_token, expires_in)
                    .map_err(|error| {
                        if used_proxy {
                            TokenRefreshError::ProxyTransient { retry_after: None }
                        } else {
                            TokenRefreshError::Unexpected(error)
                        }
                    }),
                FetchAccessTokenResponse::Error { error } => {
                    Err(TokenRefreshError::Firebase(error))
                }
            }
        })
    }

    fn fetch_access_token_via_proxy<'a>(
        client: Arc<http_client::Client>,
        request_body: &'a [(&'a str, &'a str)],
        proxy_url: String,
    ) -> BoxFuture<'a, StdResult<http_client::Response, TokenRefreshError>> {
        Box::pin(async move {
            let response = client
                .post(&proxy_url)
                .form(request_body)
                .timeout(FETCH_ACCESS_TOKEN_TIMEOUT)
                .send()
                .await
                .map_err(|_| TokenRefreshError::ProxyTransient { retry_after: None })?;
            let status = response.status();
            if status == StatusCode::TOO_MANY_REQUESTS
                || status == StatusCode::REQUEST_TIMEOUT
                || status.is_server_error()
            {
                let retry_after = response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| parse_retry_after(value, chrono::Utc::now()));
                return Err(TokenRefreshError::ProxyTransient { retry_after });
            }
            Ok(response)
        })
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
