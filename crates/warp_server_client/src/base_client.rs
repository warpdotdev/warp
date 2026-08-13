use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
use instant::Duration;
use parking_lot::{Mutex, RwLock};
use warp_graphql::client::RequestOptions;
use warp_server_auth::auth_state::AuthState;
use warp_server_auth::credentials::AuthToken;
#[cfg(feature = "agent_mode_evals")]
use warp_server_auth::credentials::Credentials;

use crate::auth::{AuthEvent, AuthSession, UserUid};

/// Header key for the ambient agent workload token attached to authenticated requests.
pub const AMBIENT_WORKLOAD_TOKEN_HEADER: &str = "X-Warp-Ambient-Workload-Token";

/// Header key for the cloud agent task ID attached to ambient-agent requests.
pub const CLOUD_AGENT_ID_HEADER: &str = "X-Warp-Cloud-Agent-ID";

/// Header used to communicate the source of an agent run.
pub const AGENT_SOURCE_HEADER: &str = "X-Oz-Api-Source";

/// IDs in the staging database that were created specifically for evals.
///
/// Keep this list in sync with `script/populate_agent_mode_eval_user.sql` in warp-server.
#[cfg(feature = "agent_mode_evals")]
const EVAL_USER_IDS: [i32; 11] = [
    2162, 2164, 2165, 2166, 2167, 2168, 2169, 2172, 2173, 2174, 2175,
];

/// Duration for which an ambient agent workload token is valid.
const AMBIENT_WORKLOAD_TOKEN_DURATION: Duration = Duration::from_secs(3 * 60 * 60);

/// Selects whether a contextual header is inherited, set, or omitted for one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeaderOverride<T> {
    Inherit,
    Set(T),
    Omit,
}

/// Describes the request-local ambient agent headers that are safe to vary by endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmbientHeaderPolicy {
    pub workload_token: HeaderOverride<String>,
    pub cloud_agent_id: HeaderOverride<String>,
    pub agent_source: HeaderOverride<String>,
}

impl AmbientHeaderPolicy {
    /// Inherits every ambient agent contextual header configured on the client.
    pub fn inherit_all() -> Self {
        Self {
            workload_token: HeaderOverride::Inherit,
            cloud_agent_id: HeaderOverride::Inherit,
            agent_source: HeaderOverride::Inherit,
        }
    }

    /// Replaces only the cloud-agent task identifier for one task-scoped request.
    pub fn for_task(task_id: impl Into<String>) -> Self {
        Self {
            cloud_agent_id: HeaderOverride::Set(task_id.into()),
            ..Self::inherit_all()
        }
    }

    /// Includes workload-token context without cloud-agent or source context.
    pub fn workload_only() -> Self {
        Self {
            workload_token: HeaderOverride::Inherit,
            cloud_agent_id: HeaderOverride::Omit,
            agent_source: HeaderOverride::Omit,
        }
    }

    /// Omits all ambient agent contextual headers for a request.
    pub fn omit_all() -> Self {
        Self {
            workload_token: HeaderOverride::Omit,
            cloud_agent_id: HeaderOverride::Omit,
            agent_source: HeaderOverride::Omit,
        }
    }
}

impl Default for AmbientHeaderPolicy {
    fn default() -> Self {
        Self::inherit_all()
    }
}

/// Provides GraphQL path routing that applies independently of authentication.
#[derive(Clone, Debug, Default)]
pub struct GraphqlRoutingConfig {
    pub path_prefix: Option<String>,
}

/// Provides headers added only to session-authenticated GraphQL operations.
#[derive(Clone, Debug, Default)]
pub struct AuthenticatedGraphqlConfig {
    pub headers: HashMap<String, String>,
}

/// Owns shared transport, authentication, and authenticated request decoration.
pub struct BaseClient {
    client: Arc<http_client::Client>,
    auth_state: Arc<AuthState>,
    event_sender: async_channel::Sender<AuthEvent>,
    auth_session: Arc<AuthSession>,
    ambient_workload_token: Arc<Mutex<Option<warp_isolation_platform::WorkloadToken>>>,
    ambient_agent_task_id: Arc<RwLock<Option<String>>>,
    agent_source: Option<String>,
    graphql_routing: GraphqlRoutingConfig,
    authenticated_graphql: AuthenticatedGraphqlConfig,
    iap_token_provider: Option<Arc<dyn http_client::iap::IapTokenProvider>>,
}

impl BaseClient {
    pub fn new(
        client: Arc<http_client::Client>,
        auth_state: Arc<AuthState>,
        event_sender: async_channel::Sender<AuthEvent>,
        agent_source: Option<String>,
        graphql_routing: GraphqlRoutingConfig,
        mut authenticated_graphql: AuthenticatedGraphqlConfig,
        iap_token_provider: Option<Arc<dyn http_client::iap::IapTokenProvider>>,
    ) -> Self {
        authenticated_graphql.headers.retain(|name, _| {
            if Self::is_reserved_authenticated_graphql_header(name) {
                log::warn!("Ignoring reserved authenticated GraphQL header configuration: {name}");
                false
            } else {
                true
            }
        });
        // We generate one random user ID per client so evals can run in parallel.
        #[cfg(feature = "agent_mode_evals")]
        let eval_user_id = {
            use rand::Rng as _;

            Some(EVAL_USER_IDS[rand::thread_rng().gen_range(0..EVAL_USER_IDS.len())])
        };
        #[cfg(feature = "agent_mode_evals")]
        if let Some(eval_user_id) = eval_user_id {
            // Set a deterministic per-user API key so all requests — including
            // REST endpoints like the SSE event stream — carry a real
            // Authorization header. The key format mirrors what SeedEvalAPIKeys()
            // inserts in warp-server at eval startup:
            // wk-1.<user_id as 64-char zero-padded lowercase hex>.
            let eval_key = format!("wk-1.{eval_user_id:0>64x}");
            auth_state.set_credentials(Some(Credentials::ApiKey {
                key: eval_key,
                owner_type: None,
            }));
        }
        let auth_session = Arc::new(AuthSession::new(
            client.clone(),
            auth_state.clone(),
            event_sender.clone(),
        ));
        Self {
            client,
            auth_state,
            event_sender,
            auth_session,
            ambient_workload_token: Arc::new(Mutex::new(None)),
            ambient_agent_task_id: Arc::new(RwLock::new(None)),
            agent_source,
            graphql_routing,
            authenticated_graphql,
            iap_token_provider,
        }
    }

    /// Returns whether authenticated GraphQL decoration would override BaseClient-owned headers.
    fn is_reserved_authenticated_graphql_header(name: &str) -> bool {
        [
            http::header::AUTHORIZATION.as_str(),
            http::header::CONTENT_TYPE.as_str(),
            http::header::CONTENT_LENGTH.as_str(),
            http_client::iap::IAP_PROXY_AUTH_HEADER,
            AMBIENT_WORKLOAD_TOKEN_HEADER,
            CLOUD_AGENT_ID_HEADER,
            AGENT_SOURCE_HEADER,
        ]
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
    }

    /// Returns the shared HTTP client for request construction.
    pub fn http_client(&self) -> &http_client::Client {
        self.client.as_ref()
    }

    /// Returns an owned handle to the shared HTTP client for GraphQL operations.
    pub fn owned_http_client(&self) -> Arc<http_client::Client> {
        self.client.clone()
    }

    pub fn auth_session(&self) -> Arc<AuthSession> {
        self.auth_session.clone()
    }

    pub fn anonymous_id(&self) -> String {
        self.auth_state.anonymous_id()
    }

    pub fn user_id(&self) -> Option<UserUid> {
        self.auth_state.user_id()
    }

    pub fn access_token_ignoring_validity(&self) -> Option<String> {
        self.auth_state.get_access_token_ignoring_validity()
    }

    pub fn allowed_to_refresh_token(&self) -> bool {
        self.auth_session.allowed_to_refresh_token()
    }

    pub async fn get_or_refresh_access_token(&self) -> Result<AuthToken> {
        self.auth_session.get_or_refresh_access_token().await
    }

    /// Returns a sender for asynchronous work that emits auth events without borrowing this client.
    pub fn event_sender(&self) -> async_channel::Sender<AuthEvent> {
        self.event_sender.clone()
    }
    /// Sends an auth event from synchronous client-owned response handling.
    pub fn send_auth_event(
        &self,
        event: AuthEvent,
    ) -> Result<(), async_channel::TrySendError<AuthEvent>> {
        self.event_sender.try_send(event)
    }

    pub fn is_auth_refresh_allowed(&self) -> bool {
        self.allowed_to_refresh_token()
    }

    /// Sets the default cloud-agent identifier inherited by subsequent requests.
    pub fn set_ambient_agent_task_id(&self, task_id: Option<String>) {
        *self.ambient_agent_task_id.write() = task_id;
    }

    /// Returns an ambient agent workload token when the current runtime can issue one.
    pub async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>> {
        if cfg!(target_family = "wasm") {
            return Ok(None);
        }
        {
            let cached = self.ambient_workload_token.lock();
            if let Some(token) = cached.as_ref() {
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

    /// Resolves request-local ambient agent policy into wire headers.
    pub async fn ambient_headers(
        &self,
        policy: AmbientHeaderPolicy,
    ) -> Result<Vec<(String, String)>> {
        let workload_token = match policy.workload_token {
            HeaderOverride::Inherit => self
                .get_or_create_ambient_workload_token()
                .await
                .context("Failed to get ambient agent workload token")?,
            HeaderOverride::Set(token) => Some(token),
            HeaderOverride::Omit => None,
        };
        let cloud_agent_id = match policy.cloud_agent_id {
            HeaderOverride::Inherit => self.ambient_agent_task_id.read().clone(),
            HeaderOverride::Set(task_id) => Some(task_id),
            HeaderOverride::Omit => None,
        };
        let agent_source = match policy.agent_source {
            HeaderOverride::Inherit => self.agent_source.clone(),
            HeaderOverride::Set(source) => Some(source),
            HeaderOverride::Omit => None,
        };

        Ok(workload_token
            .map(|token| (AMBIENT_WORKLOAD_TOKEN_HEADER.to_string(), token))
            .into_iter()
            .chain(cloud_agent_id.map(|id| (CLOUD_AGENT_ID_HEADER.to_string(), id)))
            .chain(agent_source.map(|source| (AGENT_SOURCE_HEADER.to_string(), source)))
            .collect())
    }

    /// Returns GraphQL options for bootstrap or explicit-token operations.
    pub fn graphql_request_options_with_token(&self, auth_token: Option<String>) -> RequestOptions {
        RequestOptions {
            auth_token,
            path_prefix: self.graphql_routing.path_prefix.clone(),
            ..RequestOptions::default()
        }
    }

    /// Returns GraphQL options for a session-authenticated operation.
    pub async fn graphql_request_options(
        &self,
        timeout: Option<Duration>,
    ) -> Result<RequestOptions> {
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for GraphQL request")?;
        let mut options = self.graphql_request_options_with_token(auth_token.bearer_token());
        options.timeout = timeout;
        options.headers = self.authenticated_graphql.headers.clone();
        options.headers.extend(
            self.ambient_headers(AmbientHeaderPolicy::inherit_all())
                .await?,
        );
        Ok(options)
    }

    /// Notifies the application when an enabled IAP-backed request receives an IAP challenge.
    pub fn observe_iap_challenge(&self, response: &http_client::Response) -> bool {
        if self.iap_token_provider.is_none()
            || !http_client::iap::is_iap_challenge(response.status(), response.headers())
        {
            return false;
        }
        log::warn!(
            "Received IAP challenge (status {}); notifying IapManager",
            response.status()
        );
        if let Err(error) = self.send_auth_event(AuthEvent::IapChallengeReceived) {
            log::warn!("Failed to enqueue IapChallengeReceived event: {error}");
        }
        true
    }

    /// Wraps an eventsource stream so IAP challenges notify the application without changing the
    /// original stream result or reconnecting it.
    pub fn wrap_eventsource_with_iap_detection(
        &self,
        stream: http_client::EventSourceStream,
    ) -> http_client::EventSourceStream {
        if self.iap_token_provider.is_none() {
            return stream;
        }
        let event_sender = self.event_sender();
        let wrapped = stream.map(move |event| {
            if let Err(reqwest_eventsource::Error::InvalidStatusCode(status, ref response)) = event
                && http_client::iap::is_iap_challenge(status, response.headers())
            {
                log::warn!(
                    "Received IAP challenge on eventsource (status {status}); notifying IapManager"
                );
                if let Err(error) = event_sender.try_send(AuthEvent::IapChallengeReceived) {
                    log::warn!(
                        "Failed to enqueue IapChallengeReceived event from eventsource: {error}"
                    );
                }
            }
            event
        });
        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                wrapped.boxed_local()
            } else {
                wrapped.boxed()
            }
        }
    }

    /// Inspects a WebSocket handshake error for an IAP challenge and notifies the application.
    #[cfg(not(target_family = "wasm"))]
    pub fn report_ws_iap_challenge(&self, error: &anyhow::Error) {
        if self.iap_token_provider.is_none() || !crate::iap::ws_connect_is_iap_challenge(error) {
            return;
        }
        log::warn!("Received IAP challenge on websocket handshake; notifying IapManager");
        if let Err(error) = self.send_auth_event(AuthEvent::IapChallengeReceived) {
            log::warn!("Failed to enqueue IapChallengeReceived: {error}");
        }
    }

    #[cfg(target_family = "wasm")]
    pub fn report_ws_iap_challenge(&self, _error: &anyhow::Error) {}

    /// Returns the current IAP proxy authorization header for transports outside the HTTP client.
    pub fn iap_proxy_auth_header(&self) -> Option<(&'static str, String)> {
        self.iap_token_provider
            .as_ref()?
            .cached_token()
            .map(|token| http_client::iap::proxy_auth_header(&token))
    }
}

#[cfg(test)]
#[path = "base_client_tests.rs"]
mod tests;
