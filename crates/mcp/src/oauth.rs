use std::collections::HashMap;

use futures::future::BoxFuture;
use oauth2::{RefreshToken, TokenResponse as _};
use rmcp::transport::auth::{
    AuthClient, AuthorizationManager, CredentialStore, InMemoryCredentialStore, OAuthClientConfig,
    OAuthState, StoredCredentials,
};
use rmcp::transport::{AuthError, AuthorizationSession};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;
use warp_core::channel::ChannelState;
use warp_errors::report_error;
use warpui_extras::secure_storage::AppContextExt as _;
mod loopback;

pub const TEMPLATABLE_MCP_CREDENTIALS_KEY: &str = "TemplatableMcpCredentials";
pub const FILE_BASED_MCP_CREDENTIALS_KEY: &str = "FileBasedMcpCredentials";

/// The issuer URL for GitHub's OAuth provider.
const GITHUB_ISSUER: &str = "https://github.com/login/oauth";

static GITHUB_OAUTH_SCOPES: [&str; 7] = [
    "repo",
    "read:org",
    "gist",
    "notifications",
    "user",
    "project",
    "workflow",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedCredentials {
    /// The credential information that `rmcp` wants us to store and retrieve.
    #[serde(flatten)]
    credentials: StoredCredentials,
    /// The client secret for the OAuth application.
    ///
    /// This is needed to properly refresh tokens when using DCR (Dynamic Client Registration),
    /// as the server expects the client to provide the secret when refreshing.
    client_secret: Option<String>,
}

/// Maps cloud MCP installation UUID to its OAuth credentials in secure storage.
pub type PersistedCredentialsMap = HashMap<Uuid, PersistedCredentials>;

// Maps a consistent hash of the installation to its persisted credentials
pub type FileBasedPersistedCredentialsMap = HashMap<u64, PersistedCredentials>;
pub type PersistCredentialsCallback =
    Box<dyn Fn(Uuid, PersistedCredentials) -> BoxFuture<'static, anyhow::Result<()>> + Send>;
pub type RequiresAuthenticationCallback =
    Box<dyn Fn(Uuid, String, String) -> BoxFuture<'static, anyhow::Result<()>> + Send>;
pub type AuthenticatedCallback =
    Box<dyn Fn(String) -> BoxFuture<'static, anyhow::Result<()>> + Send>;

pub enum OAuthCallbackMode {
    CustomScheme {
        redirect_uri: String,
        result_rx: async_channel::Receiver<CallbackResult>,
    },
    Loopback,
}

enum OAuthCallbackReceiver {
    CustomScheme(async_channel::Receiver<CallbackResult>),
    Loopback(loopback::LoopbackOAuthReceiver),
}

struct PreparedOAuthCallback {
    redirect_uri: String,
    receiver: OAuthCallbackReceiver,
    uses_loopback: bool,
}

impl OAuthCallbackMode {
    async fn prepare(self) -> Result<PreparedOAuthCallback, AuthError> {
        match self {
            Self::CustomScheme {
                redirect_uri,
                result_rx,
            } => Ok(PreparedOAuthCallback {
                redirect_uri,
                receiver: OAuthCallbackReceiver::CustomScheme(result_rx),
                uses_loopback: false,
            }),
            Self::Loopback => {
                let receiver = loopback::LoopbackOAuthReceiver::bind().await?;
                Ok(PreparedOAuthCallback {
                    redirect_uri: receiver.redirect_uri().to_string(),
                    receiver: OAuthCallbackReceiver::Loopback(receiver),
                    uses_loopback: true,
                })
            }
        }
    }
}

impl OAuthCallbackReceiver {
    async fn receive(self, expected_state: &str) -> Result<CallbackResult, AuthError> {
        match self {
            Self::CustomScheme(receiver) => receiver
                .recv()
                .await
                .map_err(|err| AuthError::InternalError(err.to_string())),
            Self::Loopback(receiver) => receiver.receive(expected_state).await,
        }
    }
}

/// A credential store that wraps [`InMemoryCredentialStore`] and persists token
/// updates to Warp's secure storage via a channel.
///
/// When rmcp auto-refreshes an expired access token at runtime, the rotated
/// tokens are only saved to the in-memory store by default. This wrapper
/// ensures they also get written back to secure storage so they survive app
/// restarts.
struct PersistingCredentialStore {
    inner: InMemoryCredentialStore,
    client_secret: Option<String>,
    persist_tx: async_channel::Sender<PersistedCredentials>,
}

impl PersistingCredentialStore {
    /// Per RFC 6749 §6, the authorization server MAY issue a new refresh token on
    /// refresh, but is not required to. Many OAuth providers (e.g. Figma) only
    /// issue a refresh token on the initial authorization grant and omit it from
    /// subsequent refresh responses. If we blindly persist the new token response,
    /// the refresh token is lost and the next session (or next in-process refresh)
    /// requires a full re-auth.
    ///
    /// When the new response omits a refresh token, carry forward the one already
    /// in the store. See: <https://datatracker.ietf.org/doc/html/rfc6749#section-6>
    async fn apply_refresh_token_carry_forward(&self, credentials: &mut StoredCredentials) {
        if credentials
            .token_response
            .as_ref()
            .is_none_or(|tr| tr.refresh_token().is_some())
        {
            return;
        }

        if let Some(prev_rt) = self
            .inner
            .load()
            .await
            .ok()
            .and_then(|opt| opt)
            .and_then(|prev| prev.token_response)
            .and_then(|prev_tr| prev_tr.refresh_token().cloned())
        {
            if let Some(tr) = credentials.token_response.as_mut() {
                // Carry forward the existing/previous refresh token, constructing new if needed
                tr.set_refresh_token(Some(RefreshToken::new(prev_rt.secret().to_string())));
            }
        }
    }
}

#[async_trait::async_trait]
impl CredentialStore for PersistingCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        self.inner.load().await
    }

    async fn save(&self, mut credentials: StoredCredentials) -> Result<(), AuthError> {
        self.apply_refresh_token_carry_forward(&mut credentials)
            .await;

        self.inner.save(credentials.clone()).await?;

        // Only persist credentials if we actually have any.
        if credentials.token_response.is_some() {
            let _ = self.persist_tx.try_send(PersistedCredentials {
                credentials,
                client_secret: self.client_secret.clone(),
            });
        }
        Ok(())
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.inner.clear().await
    }
}

/// Installs a [`PersistingCredentialStore`] on the given auth manager so that
/// runtime token auto-refreshes are written back to Warp's secure storage.
///
/// A background tokio task is spawned to receive credential updates and persist
/// them via the provided callback. The task terminates when the auth manager (and
/// thus the credential store's sender) is dropped.
///
/// Note: this store is not responsible for the initial population of credentials.
/// Instead, the caller seeds the inner store with any existing credentials prior
/// to installation (see [`install_persisting_credential_store`]). This store's
/// sole role is to write token updates back to secure storage as they occur.
async fn install_persisting_credential_store(
    auth_manager: &mut AuthorizationManager,
    persisted_credentials: Option<PersistedCredentials>,
    persist_credentials: PersistCredentialsCallback,
    installation_uuid: Uuid,
) {
    let client_secret = persisted_credentials
        .as_ref()
        .and_then(|c| c.client_secret.clone());
    let in_memory_store = InMemoryCredentialStore::new();

    // If we have persisted credentials, populate the backing in-memory store with them.
    if let Some(credentials) = persisted_credentials {
        let _ = in_memory_store.save(credentials.credentials).await;
    }

    let (persist_tx, persist_rx) = async_channel::unbounded();
    let store = PersistingCredentialStore {
        inner: in_memory_store,
        client_secret,
        persist_tx,
    };

    auth_manager.set_credential_store(store);

    tokio::spawn(async move {
        while let Ok(credentials) = persist_rx.recv().await {
            if let Err(err) = persist_credentials(installation_uuid, credentials).await {
                log::warn!("Failed to persist auto-refreshed MCP credentials: {err:?}");
            }
        }
    });
}

/// Context for OAuth authentication flows.
pub struct AuthContext {
    pub callback_mode: OAuthCallbackMode,
    pub uuid: Uuid,
    pub persisted_credentials: Option<PersistedCredentials>,
    /// Whether the client is running in headless/CLI mode.
    pub is_headless: bool,
    /// Whether this server was auto-discovered from a repo MCP configuration file.
    pub is_file_based: bool,
    pub persist_credentials: PersistCredentialsCallback,
    pub requires_authentication: RequiresAuthenticationCallback,
    pub authenticated: Option<AuthenticatedCallback>,
}

/// Result of OAuth callback.
#[derive(Debug, Clone)]
pub enum CallbackResult {
    Success { code: String, csrf_token: String },
    Error { error: Option<String> },
}

/// Makes an authenticated client for the given authorization server.
///
/// This takes in the URL of the resource to authenticate for, and uses that
/// to determine the authorization server.
///
/// Upon success, returns the client and a boolean indicating whether the user was required to
/// re-authenticate (e.g. re-log in).
pub async fn make_authenticated_client(
    resource_url: &str,
    http_client: reqwest::Client,
    auth_context: AuthContext,
) -> Result<(AuthClient<reqwest::Client>, bool), AuthError> {
    let AuthContext {
        callback_mode,
        uuid,
        persisted_credentials,
        is_headless,
        is_file_based,
        persist_credentials,
        requires_authentication,
        ..
    } = auth_context;

    let PreparedOAuthCallback {
        redirect_uri,
        receiver: callback_receiver,
        uses_loopback,
    } = callback_mode.prepare().await?;

    // Create the auth manager and initialize it with a backing credential store that persists
    // new credentials to secure storage.
    let client_id = persisted_credentials
        .as_ref()
        .map(|c| c.credentials.client_id.clone());
    let client_secret = persisted_credentials
        .as_ref()
        .and_then(|c| c.client_secret.clone());
    let mut auth_manager = AuthorizationManager::new(resource_url).await?;
    install_persisting_credential_store(
        &mut auth_manager,
        persisted_credentials,
        persist_credentials,
        uuid,
    )
    .await;

    // If we loaded persisted credentials from the store, and we have a valid access token
    // (or successfully refreshed a valid refresh token), we're already authorized and good
    // to go.
    if auth_manager.initialize_from_store().await? && auth_manager.get_access_token().await.is_ok()
    {
        if let (Some(client_id), Some(client_secret)) = (client_id, client_secret) {
            auth_manager.configure_client(
                OAuthClientConfig::new(client_id, redirect_uri.clone())
                    .with_client_secret(client_secret),
            )?;
        }
        return Ok((AuthClient::new(http_client, auth_manager), false));
    }

    // If we're in headless mode and we reach here, it means we either have no credentials
    // or the cached credentials failed to refresh. Block interactive OAuth in headless mode.
    if is_headless {
        if is_file_based {
            log::warn!(
                "File-based MCP server {uuid} requires OAuth authentication; \
                 skipping in headless mode. To use this server, authenticate it \
                 in the Warp desktop app first."
            );
        }
        return Err(AuthError::AuthorizationFailed(
            "MCP server requires OAuth authentication. Please authenticate this server in the \
             Warp desktop app first, then try again."
                .to_string(),
        ));
    }

    let metadata = auth_manager.discover_metadata().await?;

    // Configure the auth manager's OAuth client using dynamic or static client registration.
    let mut oauth_state = if let Some(provider) = metadata
        .issuer
        .as_deref()
        .and_then(ChannelState::mcp_oauth_provider_by_issuer)
    {
        let (client_id, client_secret) = if uses_loopback {
            let loopback_client = provider.loopback_client.ok_or_else(|| {
                AuthError::AuthorizationFailed(
                    "This OAuth provider uses a pre-registered client that is not configured for \
                     loopback callbacks. A separate loopback-capable client registration is \
                     required."
                        .to_string(),
                )
            })?;
            (loopback_client.client_id, loopback_client.client_secret)
        } else {
            (provider.client_id, Some(provider.client_secret))
        };
        // Configure the auth manager based on the static MCP configuration for this
        // issuer.
        auth_manager.set_metadata(metadata);

        let scopes = if provider.issuer == GITHUB_ISSUER {
            GITHUB_OAUTH_SCOPES
                .into_iter()
                .map(ToString::to_string)
                .collect()
        } else {
            vec![]
        };
        let mut client_config =
            OAuthClientConfig::new(client_id, redirect_uri.clone()).with_scopes(scopes);
        if let Some(client_secret) = client_secret {
            client_config = client_config.with_client_secret(client_secret);
        }
        auth_manager.configure_client(client_config)?;

        // We do a scope "upgrade" with no additional scopes here as it's the easiest way
        // to construct an auth URL.
        let auth_url = auth_manager.request_scope_upgrade("").await?;
        OAuthState::Session(AuthorizationSession::for_scope_upgrade(
            auth_manager,
            auth_url,
            &redirect_uri,
        ))
    } else {
        // Try dynamic client registration.
        let mut oauth_state = OAuthState::Unauthorized(auth_manager);
        oauth_state
            .start_authorization(&[], &redirect_uri, Some("Warp"))
            .await?;
        oauth_state
    };

    let auth_url = oauth_state.get_authorization_url().await?;

    // Extract the CSRF token that rmcp embedded as the `state` query parameter in the
    // authorization URL. We register a csrf→uuid mapping on the manager so that
    // `handle_oauth_callback` can route the callback to the right server without
    // relying on `server_id` being present in the redirect URI.
    let csrf_state = Url::parse(&auth_url)
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "state")
                .map(|(_, v)| v.into_owned())
        })
        .unwrap_or_default();

    if let Err(err) = requires_authentication(uuid, csrf_state.clone(), auth_url).await {
        log::warn!("Failed to emit RequiresAuthentication state: {err:?}");
    }

    // Wait for the authorization code from the OAuth callback channel.
    let oauth_result = callback_receiver.receive(&csrf_state).await?;

    let (code, csrf_token) = match &oauth_result {
        CallbackResult::Success { code, csrf_token } => (code, csrf_token),
        CallbackResult::Error { error } => {
            return Err(AuthError::AuthorizationFailed(
                error.as_deref().unwrap_or("unknown error").to_string(),
            ));
        }
    };

    // Handle the callback with the received authorization code and CSRF token.
    oauth_state.handle_callback(code, csrf_token).await?;

    let auth_manager = oauth_state.into_authorization_manager().ok_or_else(|| {
        AuthError::InternalError("Failed to create authorization manager".to_string())
    })?;

    Ok((AuthClient::new(http_client, auth_manager), true))
}

/// Loads credentials from secure storage at the provided key.
pub fn load_credentials_from_secure_storage<T: DeserializeOwned + Default>(
    app: &mut warpui::AppContext,
    key: &str,
) -> T {
    app.secure_storage()
        .read_value(key)
        .inspect_err(|err| {
            if !matches!(err, warpui_extras::secure_storage::Error::NotFound) {
                log::warn!("Failed to read MCP credentials from secure storage: {err:#}");
            }
        })
        .ok()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

/// Writes credentials to secure storage at the provided key.
pub fn write_to_secure_storage<T: Serialize>(
    app: &mut warpui::AppContext,
    key: &str,
    credentials: &T,
) {
    match serde_json::to_string(credentials) {
        Ok(json) => {
            if let Err(err) = app.secure_storage().write_value(key, &json) {
                report_error!(anyhow::Error::new(err)
                    .context("Failed to write MCP credentials to secure storage"));
            }
        }
        Err(err) => {
            report_error!(anyhow::Error::new(err)
                .context("Failed to serialize MCP credentials for secure storage"));
        }
    }
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
