use std::collections::HashMap;
use std::time::Duration;

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
mod coordinator;
mod loopback;

pub use coordinator::{
    AuthLease, CoordinatorConfig, CredentialKey, CredentialNamespace, OAuthCoordinator,
    OwnerMetadata, ResolveOutcome, SecureCredentialBackend, WaiterNotifier as WaiterCallback,
    forwarded_callback_looks_valid,
};

/// A desktop callback relay owned by the elected leader. The leader starts it
/// once the CSRF state is known and publishes its endpoint as owner metadata so
/// other processes can forward `warp://mcp/...` callbacks to the leader. The
/// relay validates the callback through the existing `handle_oauth_callback`
/// path and delivers exactly one result to the leader's local OAuth channel.
///
/// Methods are synchronous so cleanup can run from a `Drop` guard on every
/// terminal path (success, error, timeout, shutdown).
pub trait OwnerRelay: Send + Sync + 'static {
    /// Starts the relay for this leader flow and returns the endpoint address
    /// to publish as owner metadata. Returning an error fails closed: the
    /// leader still runs its own local callback path, but callbacks delivered
    /// to other processes will not be forwarded.
    fn start(&self, csrf_state: String) -> anyhow::Result<String>;

    /// Stops the relay and releases its resources. Called when the leader flow
    /// ends; idempotent.
    fn stop(&self);
}

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
            && let Some(tr) = credentials.token_response.as_mut()
        {
            // Carry forward the existing/previous refresh token, constructing new if needed
            tr.set_refresh_token(Some(RefreshToken::new(prev_rt.secret().to_string())));
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

/// Handle returned by [`install_persisting_credential_store`] that resolves
/// when the first credential publish through the coordinator's serialized
/// merge completes (success or failure). The leader awaits it before releasing
/// its auth lease so a follower cannot acquire the freed lease and open a
/// second OAuth page before the shared write is visible (spec invariants
/// #1/#5). Non-leader callers (cached/follower paths) simply drop it.
pub(crate) struct CredentialPublish {
    rx: tokio::sync::oneshot::Receiver<Result<(), String>>,
}

impl CredentialPublish {
    /// Waits for the first credential publish to complete, or for `timeout` to
    /// elapse. Returns `Ok(())` when the publish succeeded, `Err(msg)` when it
    /// failed, and `Err` with a timeout message when no publish happened within
    /// `timeout` (e.g. the token exchange produced no `token_response`). In
    /// every case the caller may then release the lease: the race window is
    /// closed because the publish attempt is no longer pending.
    async fn wait_for_publish(self, timeout: Duration) -> Result<(), String> {
        match tokio::time::timeout(timeout, self.rx).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(msg))) => Err(msg),
            Ok(Err(_)) => Err("credential publish task ended before publishing".to_string()),
            Err(_) => Err("timed out waiting for the credential publish".to_string()),
        }
    }
}

/// Installs a [`PersistingCredentialStore`] on the given auth manager so that
/// runtime token auto-refreshes are written back to Warp's secure storage.
///
/// A background tokio task is spawned to receive credential updates and persist
/// them via the provided callback. The task terminates when the auth manager (and
/// thus the credential store's sender) is dropped. Returns a [`CredentialPublish`]
/// that resolves when the first serialized merge/write completes, so the leader
/// can hold its lease until the credentials are durable.
///
/// Note: this store is not responsible for the initial population of credentials.
/// Instead, the caller seeds the inner store with any existing credentials prior
/// to installation (see [`install_persisting_credential_store`]). This store's
/// sole role is to write token updates back to secure storage as they occur.
async fn install_persisting_credential_store(
    auth_manager: &mut AuthorizationManager,
    persisted_credentials: Option<PersistedCredentials>,
    coordinator: std::sync::Arc<OAuthCoordinator>,
) -> CredentialPublish {
    let client_secret = persisted_credentials
        .as_ref()
        .and_then(|c| c.client_secret.clone());
    let in_memory_store = InMemoryCredentialStore::new();

    // If we have persisted credentials, populate the backing in-memory store with them.
    if let Some(credentials) = persisted_credentials {
        let _ = in_memory_store.save(credentials.credentials).await;
    }

    let (persist_tx, persist_rx) = async_channel::unbounded();
    let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
    let mut completion_tx = Some(completion_tx);
    let store = PersistingCredentialStore {
        inner: in_memory_store,
        client_secret,
        persist_tx,
    };

    auth_manager.set_credential_store(store);

    // Every credential update — the initial OAuth exchange and runtime
    // auto-refresh — goes through the coordinator's serialized read/merge/write
    // so concurrent processes and concurrent installations never overwrite one
    // another's entries in the shared secure-storage map. The first publish
    // also signals the leader's [`CredentialPublish`] so it can release its
    // lease only after the shared write is durable; subsequent refresh writes
    // do not need to signal (the leader has already released by then).
    tokio::spawn(async move {
        while let Ok(credentials) = persist_rx.recv().await {
            let result = coordinator.merge_and_write(credentials).await;
            if let Some(tx) = completion_tx.take() {
                let payload: Result<(), String> = match &result {
                    Ok(()) => Ok(()),
                    Err(err) => Err(format!("{err:#}")),
                };
                let _ = tx.send(payload);
            }
            if let Err(err) = result {
                log::warn!("Failed to persist MCP credentials to shared storage: {err:#}");
            }
        }
    });

    CredentialPublish { rx: completion_rx }
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
    /// Which shared secure-storage namespace this installation's credentials
    /// live in, and the key that identifies it inside that namespace's map.
    /// Used to build the cross-process coordinator and serialize credential
    /// writes.
    pub credential_namespace: CredentialNamespace,
    pub credential_key: CredentialKey,
    /// Access to the shared secure-storage map for this namespace, used by the
    /// coordinator for serialized read/merge/write.
    pub credential_backend: std::sync::Arc<dyn SecureCredentialBackend>,
    pub requires_authentication: RequiresAuthenticationCallback,
    /// Invoked when this process becomes a follower (waits for another instance
    /// to publish credentials) so the app can show the waiting state with no
    /// authorization URL.
    pub became_waiter: Option<WaiterCallback>,
    /// Optional desktop callback relay owned by the elected leader. `None` for
    /// the TUI loopback path (callbacks arrive on the leader's own loopback
    /// socket) and for headless/CLI flows.
    pub owner_relay: Option<std::sync::Arc<dyn OwnerRelay>>,
    pub authenticated: Option<AuthenticatedCallback>,
}

/// Result of OAuth callback.
#[derive(Debug, Clone)]
pub enum CallbackResult {
    Success { code: String, csrf_token: String },
    Error { error: Option<String> },
}

/// A stable redirect URI used only to configure the OAuth client for token
/// refresh after loading shared credentials. The authorization-code grant
/// requires a real redirect URI, but token refresh (RFC 6749 §6) does not send
/// one, so a process that loads another instance's credentials (a follower)
/// never needs to bind a callback receiver.
fn stable_refresh_redirect_uri() -> String {
    format!("{}://mcp/oauth2callback", ChannelState::url_scheme())
}

/// Attempts to build an authenticated client from cached credentials without
/// any cross-process coordination. Returns `None` when the cached credentials
/// are absent or do not yield a valid (possibly refreshed) access token, so the
/// caller falls through to the coordinated interactive flow.
async fn try_cached_client(
    resource_url: &str,
    http_client: reqwest::Client,
    credentials: PersistedCredentials,
    coordinator: std::sync::Arc<OAuthCoordinator>,
) -> Option<AuthClient<reqwest::Client>> {
    let client_id = credentials.credentials.client_id.clone();
    let client_secret = credentials.client_secret.clone();
    let mut auth_manager = AuthorizationManager::new(resource_url).await.ok()?;
    // The cached path never leads an interactive flow, so the publish handle is
    // unused; dropping it just means the background task's first-publish signal
    // is discarded.
    let _publish =
        install_persisting_credential_store(&mut auth_manager, Some(credentials), coordinator)
            .await;
    if auth_manager.initialize_from_store().await.ok()?
        && auth_manager.get_access_token().await.is_ok()
    {
        if let Some(client_secret) = client_secret {
            let _ = auth_manager.configure_client(
                OAuthClientConfig::new(client_id, stable_refresh_redirect_uri())
                    .with_client_secret(client_secret),
            );
        }
        return Some(AuthClient::new(http_client, auth_manager));
    }
    None
}

/// Builds a fresh authenticated client from shared credentials published by
/// another process (the follower path). Installs the persisting store so later
/// auto-refresh writes also go through the serialized merge. Returns
/// `did_require_login = true` so the app fires the authenticated notification
/// once for this process.
async fn build_client_from_credentials(
    resource_url: &str,
    http_client: reqwest::Client,
    credentials: PersistedCredentials,
    coordinator: std::sync::Arc<OAuthCoordinator>,
) -> Result<(AuthClient<reqwest::Client>, bool), AuthError> {
    let client_id = credentials.credentials.client_id.clone();
    let client_secret = credentials.client_secret.clone();
    let mut auth_manager = AuthorizationManager::new(resource_url).await?;
    // The follower path loaded credentials another instance published; it never
    // leads an interactive flow, so the publish handle is unused.
    let _publish =
        install_persisting_credential_store(&mut auth_manager, Some(credentials), coordinator)
            .await;
    if auth_manager.initialize_from_store().await? && auth_manager.get_access_token().await.is_ok()
    {
        if let Some(client_secret) = client_secret {
            auth_manager.configure_client(
                OAuthClientConfig::new(client_id, stable_refresh_redirect_uri())
                    .with_client_secret(client_secret),
            )?;
        }
        return Ok((AuthClient::new(http_client, auth_manager), true));
    }
    // The shared credentials did not yield a valid token (e.g. they were
    // revoked). Surface a controlled error so the spawn fails and the user can
    // retry, which re-acquires the lease and re-authenticates.
    Err(AuthError::AuthorizationFailed(
        "MCP OAuth credentials loaded from another Warp instance were not usable; \
         please retry authentication"
            .to_string(),
    ))
}

/// RAII guard that releases the auth lease, stops the desktop callback relay,
/// and removes the owner metadata on every leader terminal path (success,
/// OAuth error, callback timeout, token-exchange error, cancellation). The OS
/// also releases the lease on crash, but this guard handles the normal paths
/// and any `?` early return.
struct LeaderGuard {
    lease: Option<AuthLease>,
    coordinator: std::sync::Arc<OAuthCoordinator>,
    owner_relay: Option<std::sync::Arc<dyn OwnerRelay>>,
    relay_started: bool,
}

impl LeaderGuard {
    /// Releases all leader-held resources. Idempotent.
    fn release(&mut self) {
        if self.relay_started
            && let Some(relay) = self.owner_relay.take()
        {
            relay.stop();
        } else {
            self.owner_relay.take();
        }
        self.coordinator.remove_owner_metadata();
        self.lease.take();
    }
}

impl Drop for LeaderGuard {
    fn drop(&mut self) {
        self.release();
    }
}

/// Inputs for the leader's interactive OAuth flow, bundled to keep
/// [`run_leader_oauth`] under clippy's argument-count limit.
struct LeaderFlow {
    callback_mode: OAuthCallbackMode,
    uuid: Uuid,
    requires_authentication: RequiresAuthenticationCallback,
    owner_relay: Option<std::sync::Arc<dyn OwnerRelay>>,
}

/// Runs the interactive OAuth flow as the elected leader. The lease, relay, and
/// owner metadata are released by `LeaderGuard` on every return path.
async fn run_leader_oauth(
    resource_url: &str,
    http_client: reqwest::Client,
    coordinator: std::sync::Arc<OAuthCoordinator>,
    lease: AuthLease,
    flow: LeaderFlow,
) -> Result<(AuthClient<reqwest::Client>, bool), AuthError> {
    let LeaderFlow {
        callback_mode,
        uuid,
        requires_authentication,
        owner_relay,
    } = flow;
    let mut guard = LeaderGuard {
        lease: Some(lease),
        coordinator: coordinator.clone(),
        owner_relay,
        relay_started: false,
    };

    // Only the leader prepares the callback receiver. A follower never binds a
    // loopback port or creates a callback channel.
    let PreparedOAuthCallback {
        redirect_uri,
        receiver: callback_receiver,
        uses_loopback,
    } = callback_mode.prepare().await?;

    let mut auth_manager = AuthorizationManager::new(resource_url).await?;
    // The leader holds its lease until the credential publish is durable, so it
    // keeps the [`CredentialPublish`] handle and awaits it before `guard.release()`.
    let publish =
        install_persisting_credential_store(&mut auth_manager, None, coordinator.clone()).await;

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

    // Start the desktop callback relay (if configured) and publish owner
    // metadata so other processes can forward callbacks to this leader. The
    // relay reuses the existing `handle_oauth_callback` CSRF validation.
    //
    // For the desktop (custom-scheme) path, a relay/metadata failure fails
    // closed *before* `requires_authentication` opens the page: if a callback is
    // later delivered to a follower, it would have no published endpoint to
    // forward to and the flow would hang (spec invariants #6/#7). The TUI
    // loopback path passes `owner_relay = None`, so this only affects desktop.
    // The `?` early-return drops `LeaderGuard`, which stops the relay, removes
    // any partial metadata, and releases the lease.
    if let Some(relay) = guard.owner_relay.as_ref() {
        let address = relay.start(csrf_state.clone()).map_err(|err| {
            AuthError::InternalError(format!("failed to start MCP OAuth callback relay: {err:#}"))
        })?;
        let owner_metadata = OwnerMetadata::new(uuid, address);
        coordinator
            .publish_owner_metadata(&owner_metadata)
            .map_err(|err| {
                AuthError::InternalError(format!(
                    "failed to publish MCP OAuth owner metadata: {err:#}"
                ))
            })?;
        guard.relay_started = true;
    }

    if let Err(err) = requires_authentication(uuid, csrf_state.clone(), auth_url).await {
        log::warn!("Failed to emit RequiresAuthentication state: {err:?}");
    }

    // Wait for the authorization code from the OAuth callback channel.
    let oauth_result = callback_receiver.receive(&csrf_state).await?;

    let (code, csrf_token) = match &oauth_result {
        CallbackResult::Success { code, csrf_token } => (code.clone(), csrf_token.clone()),
        CallbackResult::Error { error } => {
            return Err(AuthError::AuthorizationFailed(
                error.as_deref().unwrap_or("unknown error").to_string(),
            ));
        }
    };

    // Handle the callback with the received authorization code and CSRF token.
    oauth_state.handle_callback(&code, &csrf_token).await?;

    let auth_manager = oauth_state.into_authorization_manager().ok_or_else(|| {
        AuthError::InternalError("Failed to create authorization manager".to_string())
    })?;

    // Wait for the credential publish to become durable *before* releasing the
    // lease, relay, and owner metadata. `handle_callback` only queues the
    // credential save onto the persisting store's background task; if we
    // released here, a follower polling in `wait_for_credentials_or_promotion`
    // could acquire the now-free lease and open a second OAuth page before the
    // shared write lands — a racy form of the exact N-page bug this flow fixes
    // (spec invariants #1/#5). The publish timeout is bounded so a stuck write
    // cannot hold the lease forever; on timeout the leader still releases (its
    // own in-memory client is valid), and the failure is logged for retry.
    let publish_timeout = coordinator
        .config()
        .cred_store_lock_timeout
        .saturating_mul(4);
    if let Err(msg) = publish.wait_for_publish(publish_timeout).await {
        log::warn!(
            "MCP OAuth leader credential publish did not complete before lease release: {msg}"
        );
    }
    guard.release();
    Ok((AuthClient::new(http_client, auth_manager), true))
}

/// Makes an authenticated client for the given authorization server, coordinating
/// across concurrent Warp processes so exactly one opens an OAuth authorization
/// page.
///
/// This takes in the URL of the resource to authenticate for, and uses that
/// to determine the authorization server.
///
/// Upon success, returns the client and a boolean indicating whether the user
/// was required to re-authenticate (e.g. re-log in), including the follower case
/// where another instance completed authentication and this process loaded the
/// shared credentials.
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
        credential_namespace,
        credential_key,
        credential_backend,
        requires_authentication,
        became_waiter,
        owner_relay,
        authenticated: _,
    } = auth_context;

    // Build the cross-process coordinator. Failing to build it fails closed for
    // interactive OAuth (a controlled error) rather than letting every process
    // open its own page.
    let channel = format!("{:?}", ChannelState::channel());
    let coordinator = std::sync::Arc::new(
        OAuthCoordinator::new(
            &channel,
            credential_namespace,
            uuid,
            credential_key,
            credential_backend,
            CoordinatorConfig::default(),
            became_waiter,
        )
        .map_err(|err| AuthError::InternalError(err.to_string()))?,
    );

    // 1. Fast path: usable cached credentials bypass coordination entirely.
    if let Some(credentials) = persisted_credentials
        && let Some(client) = try_cached_client(
            resource_url,
            http_client.clone(),
            credentials,
            coordinator.clone(),
        )
        .await
    {
        return Ok((client, false));
    }

    // 2. Headless mode never runs interactive OAuth or waits on another
    //    instance; it returns the existing non-interactive error so a later
    //    headless retry can consume credentials written by an interactive leader.
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

    // 3. Coordinate: re-check shared credentials, elect a leader, or wait for
    //    another instance to publish credentials (with promotion if it fails).
    let outcome = coordinator
        .resolve_or_wait()
        .await
        .map_err(|err| AuthError::InternalError(err.to_string()))?;
    match outcome {
        ResolveOutcome::Credentials(credentials) => {
            // Another process published credentials while we were starting or
            // while we were waiting. Build a client from them; no page opened.
            build_client_from_credentials(resource_url, http_client, *credentials, coordinator)
                .await
        }
        ResolveOutcome::Timeout => Err(AuthError::AuthorizationFailed(
            "MCP authentication in another Warp instance did not complete within the timeout; \
             please retry"
                .to_string(),
        )),
        ResolveOutcome::Leader(lease) => {
            // Re-check shared credentials under the lease: another process may
            // have published just before we acquired it. Its credentials win
            // over our stale in-memory snapshot, so we do not open a page.
            match coordinator
                .read_latest()
                .await
                .map_err(|err| AuthError::InternalError(err.to_string()))?
            {
                Some(credentials) => {
                    drop(lease);
                    build_client_from_credentials(
                        resource_url,
                        http_client,
                        credentials,
                        coordinator,
                    )
                    .await
                }
                None => {
                    run_leader_oauth(
                        resource_url,
                        http_client,
                        coordinator,
                        lease,
                        LeaderFlow {
                            callback_mode,
                            uuid,
                            requires_authentication,
                            owner_relay,
                        },
                    )
                    .await
                }
            }
        }
    }
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
                report_error!(
                    anyhow::Error::new(err)
                        .context("Failed to write MCP credentials to secure storage")
                );
            }
        }
        Err(err) => {
            report_error!(
                anyhow::Error::new(err)
                    .context("Failed to serialize MCP credentials for secure storage")
            );
        }
    }
}

/// Acquires the shared test lock that serializes tests setting the
/// `WARP_MCP_OAUTH_COORDINATION_DIR` env var, since `cargo test` runs tests in
/// parallel and process env vars are global. Hold the returned guard for the
/// lifetime of any test that touches the coordination directory. Uses
/// `tokio::sync::Mutex` so the guard may be held across `.await` points.
#[cfg(test)]
pub(crate) async fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
