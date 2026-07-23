pub mod ai;
pub mod auth;
pub mod block;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod download;
pub mod factory;
pub mod harness_support;
pub mod integrations;
pub mod managed_mcp;
pub mod managed_secrets;
pub mod object;
pub(crate) mod presigned_upload;
pub mod referral;
pub mod team;
pub mod workspace;

use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ::http::header::CONTENT_LENGTH;
use ai::AIClient;
use anyhow::{Context, Result, anyhow};
use auth::AuthClient;
use block::BlockClient;
use channel_versions::ChannelVersions;
use chrono::{DateTime, FixedOffset};
use factory::FactoryClient;
use instant::Instant;
use managed_mcp::ManagedMcpClient;
use object::ObjectClient;
use parking_lot::Mutex;
use referral::ReferralsClient;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use team::TeamClient;
use url::Url;
use warp_core::context_flag::ContextFlag;
use warp_core::telemetry::TelemetryEvent;
use warp_errors::{AnyhowErrorExt, ErrorExt, register_error, report_error};
use warp_managed_secrets::client::ManagedSecretsClient;
use warp_server_client::auth::{AuthClientImpl, AuthEvent, EXPERIMENT_ID_HEADER};
use warp_server_client::base_client::{
    AmbientHeaderPolicy, AuthenticatedGraphqlConfig, BaseClient, GraphqlRoutingConfig,
};
use warp_server_client::iap::{IapManager, IapState};
use warp_server_client::network_logging::NetworkLogModel;
use warpui::r#async::BoxFuture;
use warpui::{Entity, ModelContext, SingletonEntity};
use workspace::WorkspaceClient;

use super::experiments::{ServerExperiment, ServerExperiments};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::get_relevant_files::api::{GetRelevantFiles, GetRelevantFilesResponse};
use crate::ai::predict::generate_ai_input_suggestions::GenerateAIInputSuggestionsRequest;
use crate::ai::predict::generate_am_query_suggestions::GenerateAMQuerySuggestionsRequest;
use crate::ai::predict::predict_am_queries::{PredictAMQueriesRequest, PredictAMQueriesResponse};
use crate::ai::predict::{generate_ai_input_suggestions, generate_am_query_suggestions};
use crate::ai::voice::transcribe::{TranscribeRequest, TranscribeResponse};
use crate::auth::auth_manager::AuthManager;
use crate::auth::auth_state::AuthState;
use crate::server::telemetry::TelemetryApi;
use crate::settings::PrivacySettingsSnapshot;
use crate::{ChannelState, settings_view};

pub const FETCH_CHANNEL_VERSIONS_TIMEOUT: std::time::Duration = Duration::from_secs(60);
#[derive(Serialize)]
struct AgentTipShownAnalyticsRequest {
    tip: String,
}

/// We use a special error code header `X-Warp-Error-Code` to allow the server to send
/// more specific error code information, so that the client can discern between different
/// errors with the same error code.
/// See errors/http_error_codes.go on the server for possible values.
const WARP_ERROR_CODE_HEADER: &str = "X-Warp-Error-Code";

/// An error indicating the user is out of credits. The server sends 429s to communicate this
/// state, but if Cloud Run is overloaded, it can also send 429s that aren't credit-related.
/// So we use this to distinguish between the two cases.
const WARP_ERROR_CODE_OUT_OF_CREDITS: &str = "OUT_OF_CREDITS";

/// Error code indicating the user has reached their cloud agent concurrency limit.
const WARP_ERROR_CODE_AT_CAPACITY: &str = "AT_CLOUD_AGENT_CAPACITY";

/// ResponseType received by Client
#[derive(thiserror::Error, Debug, Serialize, Deserialize)]
#[error("{error}")]
pub struct ClientError {
    pub error: String,
    // We unconditionally check for GitHub auth errors in any public API response. It'd be much better
    // to have the server return error codes that we can parse, but this isn't yet supported.
    // See REMOTE-666
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
}

impl Deref for ServerApi {
    type Target = BaseClient;

    fn deref(&self) -> &Self::Target {
        &self.base_client
    }
}

/// Error when the user is at their cloud agent concurrency limit.
#[derive(thiserror::Error, Debug, Clone, Deserialize)]
#[error("{error} (running agents: {running_agents})")]
pub struct CloudAgentCapacityError {
    pub error: String,
    pub running_agents: i32,
}

#[derive(Deserialize, Debug)]
struct TimeResponse {
    current_time: DateTime<FixedOffset>,
}

#[derive(Debug, Clone)]
pub struct ServerTime {
    time_at_fetch: DateTime<FixedOffset>,
    fetched_at: Instant,
}

impl ServerTime {
    pub fn current_time(&self) -> DateTime<FixedOffset> {
        let elapsed = chrono::Duration::from_std(self.fetched_at.elapsed())
            .expect("duration should not be bigger than limit");
        self.time_at_fetch + elapsed
    }
}

/// Wrapper for deserialization errors. This covers both:
/// * Using `serde` directly
/// * Using `reqwest` decoding utilities
#[derive(thiserror::Error, Debug)]
pub enum DeserializationError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Transport(reqwest::Error),
}

#[derive(Deserialize, Debug)]
struct OutOfCreditsResponse {
    #[serde(default, rename = "userDisplayMessage")]
    user_display_message: Option<String>,
}

#[derive(thiserror::Error, Debug)]
pub enum AIApiError {
    #[error("Request failed due to lack of AI quota.")]
    QuotaLimit {
        user_display_message: Option<String>,
    },

    #[error("Warp is currently overloaded. Please try again later.")]
    ServerOverloaded,

    #[error("Internal error occurred at transport layer.")]
    Transport(#[source] reqwest::Error),

    #[error("Failed to deserialize API response.")]
    Deserialization(#[source] DeserializationError),

    #[error("No context found on context search.")]
    NoContextFound,

    #[error("Failed with status code {0}: {1}")]
    ErrorStatus(http::StatusCode, String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),

    #[error("Got error when streaming {stream_type}: {source:#}")]
    Stream {
        stream_type: &'static str,
        #[source]
        source: anyhow::Error,
    },

    /// Synthesized client-side when a response stream ends without a stream-finished
    /// event: the server always sends one, but the transport can truncate the response
    /// between chunks, surfacing as a clean EOF.
    #[error("Response stream ended unexpectedly before completion.")]
    UnexpectedEof,

    /// Synthesized client-side when a request that uses the connected Grok
    /// subscription can't be sent because its expired OAuth token failed to
    /// refresh. Surfaced as a terminal, user-visible error asking the user to
    /// reconnect, rather than sending a request that would fail authentication.
    #[error(
        "Grok subscription token could not be refreshed. Please try reconnecting your subscription."
    )]
    GrokSubscriptionTokenRefreshFailed,
}

impl From<http_client::ResponseError> for AIApiError {
    fn from(err: http_client::ResponseError) -> Self {
        let http_client::ResponseError {
            source,
            headers,
            body,
        } = err;
        Self::from_response_error(source, &headers, body)
    }
}

impl From<reqwest::Error> for AIApiError {
    fn from(err: reqwest::Error) -> Self {
        Self::from_transport_error(err)
    }
}

impl From<serde_json::Error> for AIApiError {
    fn from(err: serde_json::Error) -> Self {
        AIApiError::Deserialization(err.into())
    }
}

impl AIApiError {
    /// Converts a reqwest error to an AIApiError, using response headers to distinguish
    /// between different types of 429 errors.
    fn from_response_error(
        err: reqwest::Error,
        headers: &::http::HeaderMap,
        body: Option<String>,
    ) -> Self {
        // For HTTP 429 errors, check the X-Warp-Error-Code header to distinguish
        // between out-of-credits and server-overload.
        if err.status() == Some(http::StatusCode::TOO_MANY_REQUESTS) {
            return Self::error_for_429(headers, body);
        }

        Self::from_transport_error(err)
    }

    /// Converts a transport-level reqwest error (no HTTP response) to an AIApiError.
    fn from_transport_error(err: reqwest::Error) -> Self {
        // Unfortunately, `reqwest` reports some non-decoding errors as decoding errors (e.g.
        // unexpected disconnects or timeouts while deserializing a response body). Since we
        // render deserialization and transport errors differently, we try to detect those cases
        // here.
        if err.is_timeout() {
            return AIApiError::Transport(err);
        }
        if err.is_decode() {
            #[cfg(not(target_family = "wasm"))]
            {
                use std::error::Error as _;
                let mut source = err.source();
                while let Some(underlying) = source {
                    if underlying.is::<hyper::Error>() {
                        return AIApiError::Transport(err);
                    }

                    source = underlying.source();
                }
            }

            return AIApiError::Deserialization(DeserializationError::Transport(err));
        }

        AIApiError::Transport(err)
    }

    /// Returns the appropriate error for a 429 response by checking the X-Warp-Error-Code header.
    fn error_for_429(headers: &::http::HeaderMap, body: Option<String>) -> Self {
        if headers
            .get(WARP_ERROR_CODE_HEADER)
            .and_then(|v| v.to_str().ok())
            == Some(WARP_ERROR_CODE_OUT_OF_CREDITS)
        {
            let user_display_message = body
                .and_then(|body| serde_json::from_str::<OutOfCreditsResponse>(&body).ok())
                .and_then(|r| r.user_display_message);
            AIApiError::QuotaLimit {
                user_display_message,
            }
        } else {
            AIApiError::ServerOverloaded
        }
    }

    /// Format a stream error into a human-readable error message. This will read the response
    /// body if there is one.
    pub(crate) async fn from_stream_error(
        stream_type: &'static str,
        err: reqwest_eventsource::Error,
    ) -> Self {
        match err {
            reqwest_eventsource::Error::InvalidStatusCode(
                http::StatusCode::TOO_MANY_REQUESTS,
                res,
            ) => {
                let headers = res.headers().clone();
                let body = res.text().await.ok();
                Self::error_for_429(&headers, body)
            }
            reqwest_eventsource::Error::InvalidStatusCode(status, res) => Self::ErrorStatus(
                status,
                res.text()
                    .await
                    .unwrap_or_else(|e| format!("(no response body: {e:#})")),
            ),
            reqwest_eventsource::Error::Transport(err) => Self::from_transport_error(err),
            err => AIApiError::Stream {
                stream_type,
                // On WASM, `reqwest_eventsource::Error` doesn't implement `Into<anyhow::Error>` or
                // `Send` because it may contain a `wasm_bindgen` JS value.
                #[cfg(target_family = "wasm")]
                source: anyhow!("{err:#?}"),
                #[cfg(not(target_family = "wasm"))]
                source: anyhow!(err),
            },
        }
    }

    /// Whether the error is worth an automatic recovery attempt — a fresh request may
    /// succeed. Gates both retry (pre-actions) and resume (post-actions).
    pub fn is_recoverable(&self) -> bool {
        // Don't recover from client errors, except timeouts and rate limits.
        fn is_recoverable_status(status: http::StatusCode) -> bool {
            !status.is_client_error()
                || status == http::StatusCode::REQUEST_TIMEOUT
                || status == http::StatusCode::TOO_MANY_REQUESTS
        }

        match self {
            AIApiError::ErrorStatus(status, _) => is_recoverable_status(*status),
            AIApiError::Transport(e) => {
                if let Some(status) = e.status() {
                    return is_recoverable_status(status);
                }
                true
            }
            // A failed Grok token refresh is a credential problem the user must
            // fix by reconnecting, so retrying or resuming won't help.
            AIApiError::GrokSubscriptionTokenRefreshFailed => false,
            // By default, attempt recovery on error.
            _ => true,
        }
    }
}

impl ErrorExt for AIApiError {
    fn is_actionable(&self) -> bool {
        match self {
            AIApiError::Deserialization(_) => true,
            AIApiError::Transport(error) => error.is_actionable(),
            AIApiError::Other(error) => error.is_actionable(),
            AIApiError::Stream { source, .. } => source.is_actionable(),
            AIApiError::ErrorStatus(_, _) => self.is_recoverable(),
            AIApiError::UnexpectedEof => true,
            AIApiError::QuotaLimit { .. }
            | AIApiError::ServerOverloaded
            | AIApiError::NoContextFound
            | AIApiError::GrokSubscriptionTokenRefreshFailed => false,
        }
    }
}
register_error!(AIApiError);

#[derive(thiserror::Error, Debug)]
pub enum TranscribeError {
    #[error("Request failed due to lack of Voice quota.")]
    QuotaLimit,

    #[error("Warp is currently overloaded. Please try again later.")]
    ServerOverloaded,

    #[error("Internal error occurred at transport layer.")]
    Transport,

    #[error("Failed to deserialize JSON.")]
    Deserialization,

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// An API wrapper struct with methods to requests to warp-server.
///
/// Prefer NOT adding new methods directly on this struct; instead, add to one of the existing
/// client trait objects, or create your own. This helps keep `ServerApi` from being overloaded
/// with disparate types of calls, and allows you to mock methods in tests.
pub struct ServerApi {
    base_client: Arc<BaseClient>,
    // TODO(jeff): Make `TelemetryApi` another type of client, and move it off `ServerApi`.
    telemetry_api: TelemetryApi,
    last_server_time: Arc<Mutex<Option<ServerTime>>>,
}

impl ServerApi {
    fn new(
        auth_state: Arc<AuthState>,
        event_sender: async_channel::Sender<AuthEvent>,
        agent_source: Option<ai::AgentSource>,
        iap_state: Option<Arc<IapState>>,
        ctx: &mut ModelContext<ServerApiProvider>,
    ) -> Self {
        let mut client = http_client::Client::new();
        let iap_token_provider = iap_state.map(|state| {
            client.set_iap_token_provider(state.clone());
            state as Arc<dyn http_client::iap::IapTokenProvider>
        });
        let mut telemetry_api = TelemetryApi::new();
        if ContextFlag::NetworkLogConsole.is_enabled() {
            NetworkLogModel::handle(ctx).update(ctx, |model, model_ctx| {
                model.install_on_clients([&mut client, &mut telemetry_api.client], model_ctx);
            });
        }
        Self::new_with_parts(
            Arc::new(client),
            auth_state,
            event_sender,
            agent_source,
            iap_token_provider,
            telemetry_api,
        )
    }

    fn new_with_parts(
        client: Arc<http_client::Client>,
        auth_state: Arc<AuthState>,
        event_sender: async_channel::Sender<AuthEvent>,
        agent_source: Option<ai::AgentSource>,
        iap_token_provider: Option<Arc<dyn http_client::iap::IapTokenProvider>>,
        telemetry_api: TelemetryApi,
    ) -> Self {
        let graphql_routing = GraphqlRoutingConfig {
            #[cfg(feature = "agent_mode_evals")]
            path_prefix: Some("/agent-mode-evals".to_string()),
            #[cfg(not(feature = "agent_mode_evals"))]
            path_prefix: None,
        };
        let authenticated_graphql = AuthenticatedGraphqlConfig::default();
        let base_client = Arc::new(BaseClient::new(
            client,
            auth_state,
            event_sender,
            agent_source.map(|source| source.as_str().to_string()),
            graphql_routing,
            authenticated_graphql,
            iap_token_provider,
        ));

        Self {
            base_client,
            telemetry_api,
            last_server_time: Arc::new(Mutex::new(None)),
        }
    }

    #[cfg(any(test, all(feature = "tui", feature = "test-util")))]
    fn new_for_test() -> Self {
        let (tx, _) = async_channel::unbounded();
        let auth_state = Arc::new(AuthState::new_for_test());
        let client = Arc::new(http_client::Client::new_for_test());

        Self::new_with_parts(client, auth_state, tx, None, None, TelemetryApi::new())
    }

    #[cfg(all(test, feature = "skip_login"))]
    fn new_for_test_with_bearer_token(
        bearer_token: Option<String>,
        event_sender: async_channel::Sender<AuthEvent>,
    ) -> Self {
        let auth_state = Arc::new(AuthState::new_logged_out_for_test());
        if let Some(bearer_token) = bearer_token {
            auth_state.set_remote_server_bearer_token(bearer_token);
        }
        Self::new_with_parts(
            Arc::new(http_client::Client::new_for_test()),
            auth_state,
            event_sender,
            None,
            None,
            TelemetryApi::new(),
        )
    }

    /// Sets the ambient agent task ID to be sent with all subsequent requests.
    pub fn set_ambient_agent_task_id(&self, task_id: Option<AmbientAgentTaskId>) {
        self.base_client
            .set_ambient_agent_task_id(task_id.map(|task_id| task_id.to_string()));
    }

    /// Returns ambient agent headers to attach to requests.
    async fn ambient_agent_headers(&self) -> Result<Vec<(String, String)>> {
        self.ambient_headers(AmbientHeaderPolicy::inherit_all())
            .await
    }

    async fn ambient_agent_headers_for_task(
        &self,
        task_id: &AmbientAgentTaskId,
    ) -> Result<Vec<(String, String)>> {
        self.ambient_headers(AmbientHeaderPolicy::for_task(task_id.to_string()))
            .await
    }

    pub fn send_graphql_request<'a, QF, O: warp_graphql::client::Operation<QF> + Send + 'a>(
        &'a self,
        operation: O,
        timeout: Option<Duration>,
    ) -> BoxFuture<'a, Result<QF>>
    where
        QF: 'a,
    {
        warp_server_client::graphql_helpers::send_graphql_request(
            &self.base_client,
            operation,
            timeout,
        )
    }

    /// Opens an SSE stream to the agent event-push endpoint.
    ///
    /// The returned `EventSourceStream` yields `reqwest_eventsource::Event`
    /// items until the connection closes or an error occurs. The caller is
    /// responsible for reading the stream and handling reconnection.
    ///
    /// The stream is served by warp-server-rtc (not the main warp-server pool),
    /// so the URL is built from `ChannelState::rtc_http_url()` rather than
    /// `server_root_url()`.
    pub async fn stream_agent_events(
        &self,
        run_ids: &[String],
        since_sequence: i64,
    ) -> Result<http_client::EventSourceStream> {
        debug_assert!(!run_ids.is_empty(), "run_ids must not be empty");
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for SSE stream")?;

        let run_ids_param: String = run_ids
            .iter()
            .map(|id| format!("run_ids[]={}", urlencoding::encode(id)))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!(
            "{}/api/v1/agent/events/stream?{run_ids_param}&since={since_sequence}",
            ChannelState::rtc_http_url()
        );

        let mut request = self.base_client.http_client().get(&url);
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }

        for (name, value) in self.ambient_agent_headers().await? {
            request = request.header(name, value);
        }

        Ok(self.wrap_eventsource_with_iap_detection(request.eventsource()))
    }

    /// Opens an SSE stream against the ancestor-scoped agent event endpoint.
    pub async fn stream_agent_events_for_ancestor(
        &self,
        ancestor_run_id: &str,
        include_self: bool,
        since_sequence: i64,
    ) -> Result<http_client::EventSourceStream> {
        debug_assert!(
            !ancestor_run_id.is_empty(),
            "ancestor_run_id must not be empty"
        );
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for SSE stream")?;

        let include_self_param = if include_self {
            "&include_self=true"
        } else {
            ""
        };
        let url = format!(
            "{}/api/v1/agent/events/stream?ancestor_run_id={}&since={since_sequence}{include_self_param}",
            ChannelState::rtc_http_url(),
            urlencoding::encode(ancestor_run_id),
        );

        let mut request = self.base_client.http_client().get(&url);
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }

        for (name, value) in self.ambient_agent_headers().await? {
            request = request.header(name, value);
        }

        Ok(self.wrap_eventsource_with_iap_detection(request.eventsource()))
    }

    pub async fn stream_agent_events_for_task(
        &self,
        task_id: &AmbientAgentTaskId,
        run_ids: &[String],
        since_sequence: i64,
    ) -> Result<http_client::EventSourceStream> {
        debug_assert!(!run_ids.is_empty(), "run_ids must not be empty");
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for SSE stream")?;

        let run_ids_param: String = run_ids
            .iter()
            .map(|id| format!("run_ids[]={}", urlencoding::encode(id)))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!(
            "{}/api/v1/agent/events/stream?{run_ids_param}&since={since_sequence}",
            ChannelState::rtc_http_url()
        );

        let mut request = self.base_client.http_client().get(&url);
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }

        for (name, value) in self.ambient_agent_headers_for_task(task_id).await? {
            request = request.header(name, value);
        }

        Ok(self.wrap_eventsource_with_iap_detection(request.eventsource()))
    }

    /// Sends a POST request to a public API endpoint and returns the raw response on success.
    async fn post_public_api_response<B>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<http_client::Response>
    where
        B: Serialize,
    {
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for API request")?;

        let url = format!("{}/api/v1/{}", ChannelState::server_root_url(), path);

        let mut request = self.base_client.http_client().post(&url).json(body);
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }

        for (name, value) in self.ambient_agent_headers().await? {
            request = request.header(name, value);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to send API request to {url}"))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            self.observe_iap_challenge(&response);
            Err(Self::error_from_response(response).await)
        }
    }

    /// Converts a non-success public API response into the most specific client error available.
    async fn error_from_response(response: http_client::Response) -> anyhow::Error {
        let status = response.status();
        let is_at_capacity = response
            .headers()
            .get(WARP_ERROR_CODE_HEADER)
            .and_then(|v| v.to_str().ok())
            == Some(WARP_ERROR_CODE_AT_CAPACITY);
        let is_out_of_credits = response
            .headers()
            .get(WARP_ERROR_CODE_HEADER)
            .and_then(|v| v.to_str().ok())
            == Some(WARP_ERROR_CODE_OUT_OF_CREDITS);

        // Get the response text first since we may need to try multiple deserializations.
        let response_text = response.text().await.unwrap_or_default();

        // Check for AT_CAPACITY error code header.
        if is_at_capacity
            && let Ok(capacity_error) =
                serde_json::from_str::<CloudAgentCapacityError>(&response_text)
        {
            return capacity_error.into();
        }
        if status == StatusCode::TOO_MANY_REQUESTS && is_out_of_credits {
            let user_display_message = serde_json::from_str::<OutOfCreditsResponse>(&response_text)
                .ok()
                .and_then(|r| r.user_display_message);
            return AIApiError::QuotaLimit {
                user_display_message,
            }
            .into();
        }

        // Try to deserialize error response as { "error": "message" }
        match serde_json::from_str::<ClientError>(&response_text) {
            Ok(error_response) => error_response.into(),
            Err(_) => anyhow!("API request failed with status {status}"),
        }
    }

    /// Sends a POST request to a public API endpoint.
    ///
    /// # Arguments
    /// * `path` - Endpoint path relative to `/api/v1` (e.g., "agent/run")
    /// * `body` - Request body to serialize as JSON
    async fn post_public_api<B, R>(&self, path: &str, body: &B) -> Result<R>
    where
        B: Serialize,
        R: serde::de::DeserializeOwned,
    {
        let response = self.post_public_api_response(path, body).await?;
        let url = response.url().clone();
        response
            .json::<R>()
            .await
            .with_context(|| format!("Failed to deserialize response from {url}"))
    }

    /// Sends a PUT request to a public API endpoint and returns the raw response on success.
    async fn put_public_api_response<B>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<http_client::Response>
    where
        B: Serialize,
    {
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for API request")?;

        let url = format!("{}/api/v1/{}", ChannelState::server_root_url(), path);

        let mut request = self.base_client.http_client().put(&url).json(body);
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }

        for (name, value) in self.ambient_agent_headers().await? {
            request = request.header(name, value);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to send API request to {url}"))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            Err(Self::error_from_response(response).await)
        }
    }

    /// Sends a PUT request to a public API endpoint.
    async fn put_public_api<B, R>(&self, path: &str, body: &B) -> Result<R>
    where
        B: Serialize,
        R: serde::de::DeserializeOwned,
    {
        let response = self.put_public_api_response(path, body).await?;
        let url = response.url().clone();
        response
            .json::<R>()
            .await
            .with_context(|| format!("Failed to deserialize response from {url}"))
    }

    /// Sends a POST request to a public API endpoint that returns no response body.
    async fn post_public_api_unit<B>(&self, path: &str, body: &B) -> Result<()>
    where
        B: Serialize,
    {
        self.post_public_api_response(path, body).await?;
        Ok(())
    }

    /// Sends a DELETE request to a public API endpoint that returns no response body.
    async fn delete_public_api_unit(&self, path: &str) -> Result<()> {
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for API request")?;

        let url = format!("{}/api/v1/{}", ChannelState::server_root_url(), path);

        let mut request = self.base_client.http_client().delete(&url);
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }

        for (name, value) in self.ambient_agent_headers().await? {
            request = request.header(name, value);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to send API request to {url}"))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(Self::error_from_response(response).await)
        }
    }

    /// Sends a PATCH request to a public API endpoint that returns no response body.
    async fn patch_public_api_unit<B>(&self, path: &str, body: &B) -> Result<()>
    where
        B: Serialize,
    {
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for API request")?;

        let url = format!("{}/api/v1/{}", ChannelState::server_root_url(), path);

        let mut request = self.base_client.http_client().patch(&url).json(body);
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }

        for (name, value) in self.ambient_agent_headers().await? {
            request = request.header(name, value);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to send API request to {url}"))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(Self::error_from_response(response).await)
        }
    }

    /// Sends an authenticated empty POST request to /client/login, which signals to the server
    /// that the user is logged in.
    pub async fn notify_login(&self) {
        match self.get_or_refresh_access_token().await {
            Ok(auth_token) => {
                let url = format!("{}/client/login", ChannelState::server_root_url());
                let mut request = self.base_client.http_client().post(&url);
                if let Some(token) = auth_token.as_bearer_token() {
                    request = request.bearer_auth(token);
                }
                request = request
                    // Set the content-length header to 0 because the request has no body.
                    // Otherwise, the server will return a 411 error. (In other cases, setting
                    // content-type is sufficient (elides the content-length requirement), but
                    // since this request has no body, it makes more sense to set content-length.
                    .header(CONTENT_LENGTH, 0)
                    .header(EXPERIMENT_ID_HEADER, self.anonymous_id());

                let response = request.send().await;
                if let Err(err) = response {
                    report_error!(
                        anyhow::Error::new(err)
                            .context("Failed to send POST request to /client/login")
                    );
                }
            }
            Err(err) => {
                report_error!(
                    err.context("Could not retrieve access token for notifying user login")
                );
            }
        }
    }

    /// Synchronously sends a [`TelemetryEvent`] to the Rudderstack API. Prefer not to call this
    /// directly, use the macros defined in crate::server::telemetry::macros. If telemetry is
    /// disabled, this is a no-op.
    pub async fn send_telemetry_event(
        &self,
        event: impl TelemetryEvent,
        settings_snapshot: PrivacySettingsSnapshot,
    ) -> Result<()> {
        let user_id = self.user_id();
        let anonymous_id = self.anonymous_id();
        self.telemetry_api
            .send_telemetry_event(user_id, anonymous_id, event, settings_snapshot)
            .await
    }

    pub async fn send_agent_tip_shown_analytics_event(&self, tip: String) -> Result<()> {
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for API request")?;
        let url = format!(
            "{}/analytics/agent-tip-shown",
            ChannelState::server_root_url()
        );
        let mut request = self
            .base_client
            .http_client()
            .post(&url)
            .json(&AgentTipShownAnalyticsRequest { tip });
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }

        for (name, value) in self.ambient_agent_headers().await? {
            request = request.header(name, value);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to send API request to {url}"))?;

        if response.status().is_success() {
            Ok(())
        } else {
            self.observe_iap_challenge(&response);
            Err(Self::error_from_response(response).await)
        }
    }

    /// Drains all queued [`TelemetryEvent`]s into Rudderstack requests containing the corresponding
    /// batch of events. Events are queued using the [`send_telemetry_from_ctx`] or
    /// [`send_telemetry_from_app_ctx`] macros. If telemetry is disabled for the user, this flushes
    /// the UI framework event queue and does nothing with them (no request is made).
    ///
    /// Returns the number of events that were flushed.
    pub async fn flush_telemetry_events(
        &self,
        settings_snapshot: PrivacySettingsSnapshot,
    ) -> Result<usize> {
        self.telemetry_api.flush_events(settings_snapshot).await
    }

    /// Sends a batched Rudder request containing events written to the file at `path`. This is a
    /// no-op if telemetry is disabled.
    pub async fn flush_persisted_events_to_rudder(
        &self,
        path: &Path,
        settings_snapshot: PrivacySettingsSnapshot,
    ) -> Result<()> {
        self.telemetry_api
            .flush_persisted_events_to_rudder(path, settings_snapshot)
            .await
    }

    /// Writes all queued [`TelemetryEvent`]s to a file, limiting the number of written
    /// events to `max_events`. Events are queued using the [`send_telemetry_from_ctx`] or
    /// [`send_telemetry_from_app_ctx`] macros. If telemetry is disabled, no events are written to
    /// disk.
    pub fn persist_telemetry_events(
        &self,
        max_event_count: usize,
        settings_snapshot: PrivacySettingsSnapshot,
    ) -> Result<()> {
        self.telemetry_api
            .flush_and_persist_events(max_event_count, settings_snapshot)
    }

    /// Hits the /ai/generate_input_suggestions endpoint to get the predicted next action, based on past context.
    pub async fn generate_ai_input_suggestions(
        &self,
        request: &GenerateAIInputSuggestionsRequest,
    ) -> Result<generate_ai_input_suggestions::GenerateAIInputSuggestionsResponseV2, AIApiError>
    {
        let auth_token = self.get_or_refresh_access_token().await?;

        let request_builder = self.base_client.http_client().post(format!(
            "{}/ai/generate_input_suggestions",
            ChannelState::server_root_url()
        ));
        let response = if let Some(token) = auth_token.as_bearer_token() {
            request_builder.bearer_auth(token)
        } else {
            request_builder
        }
        .json(request)
        .send()
        .await?
        .error_for_status_with_body()
        .await?
        .json()
        .await?;
        Ok(response)
    }

    pub async fn get_relevant_files(
        &self,
        request: &GetRelevantFiles,
    ) -> Result<GetRelevantFilesResponse, AIApiError> {
        let auth_token = self.get_or_refresh_access_token().await?;

        let request_builder = self.base_client.http_client().post(format!(
            "{}/ai/relevant_files",
            ChannelState::server_root_url()
        ));
        let response = if let Some(token) = auth_token.as_bearer_token() {
            request_builder.bearer_auth(token)
        } else {
            request_builder
        }
        .json(request)
        .send()
        .await?
        .error_for_status_with_body()
        .await?
        .json()
        .await?;

        Ok(response)
    }

    /// Hits the /ai/generate_am_query_suggestions endpoint to get the predicted next query.
    pub async fn generate_am_query_suggestions(
        &self,
        request: &GenerateAMQuerySuggestionsRequest,
    ) -> Result<generate_am_query_suggestions::GenerateAMQuerySuggestionsResponse, AIApiError> {
        let auth_token = self.get_or_refresh_access_token().await?;

        cfg_if::cfg_if! {
            if #[cfg(feature = "agent_mode_evals")] {
                let url = format!(
                    "{}/agent-mode-evals/generate_am_query_suggestions",
                    ChannelState::server_root_url()
                );
            } else {
                let url = format!(
                    "{}/ai/generate_am_query_suggestions",
                    ChannelState::server_root_url()
                );
            }
        }

        let request_builder = self.base_client.http_client().post(url);
        let response = if let Some(token) = auth_token.as_bearer_token() {
            request_builder.bearer_auth(token)
        } else {
            request_builder
        }
        .json(request)
        .send()
        .await?
        .error_for_status_with_body()
        .await?
        .json()
        .await?;
        Ok(response)
    }

    pub async fn predict_am_queries(
        &self,
        request: &PredictAMQueriesRequest,
    ) -> Result<PredictAMQueriesResponse, AIApiError> {
        let auth_token = self.get_or_refresh_access_token().await?;
        let request_builder = self.base_client.http_client().post(format!(
            "{}/ai/predict_am_queries",
            ChannelState::server_root_url()
        ));
        let response = if let Some(token) = auth_token.as_bearer_token() {
            request_builder.bearer_auth(token)
        } else {
            request_builder
        }
        .json(request)
        .send()
        .await?
        .error_for_status_with_body()
        .await?
        .json()
        .await?;
        Ok(response)
    }

    /// Hits the /ai/transcribe endpoint to get the transcription for the given audio.
    pub async fn transcribe(
        &self,
        request: &TranscribeRequest,
    ) -> Result<TranscribeResponse, TranscribeError> {
        let auth_token = self.get_or_refresh_access_token().await?;

        let request_builder = self
            .base_client
            .http_client()
            .post(format!("{}/ai/transcribe", ChannelState::server_root_url()));
        let response = if let Some(token) = auth_token.as_bearer_token() {
            request_builder.bearer_auth(token)
        } else {
            request_builder
        }
        .json(request)
        .send()
        .await;

        match response {
            Ok(res) => {
                if res.status().is_success() {
                    match res.json::<TranscribeResponse>().await {
                        Ok(output_response) => Ok(output_response),
                        Err(e) => {
                            log::warn!("Failed to deserialize response: {e:?}");
                            Err(TranscribeError::Deserialization)
                        }
                    }
                } else if res.status() == http::StatusCode::TOO_MANY_REQUESTS {
                    if res
                        .headers()
                        .get(WARP_ERROR_CODE_HEADER)
                        .and_then(|v| v.to_str().ok())
                        == Some(WARP_ERROR_CODE_OUT_OF_CREDITS)
                    {
                        Err(TranscribeError::QuotaLimit)
                    } else {
                        Err(TranscribeError::ServerOverloaded)
                    }
                } else {
                    log::warn!("Non-success status code received: {}", res.status());
                    Err(TranscribeError::Transport)
                }
            }
            Err(e) => {
                log::warn!("Error while sending request: {e:?}");
                Err(TranscribeError::Transport)
            }
        }
    }

    fn set_server_time(&self, server_time: ServerTime) {
        let mut last_server_time = self.last_server_time.lock();
        *last_server_time = Some(server_time);
    }

    fn cached_server_time(&self) -> Option<ServerTime> {
        let last_server_time = self.last_server_time.lock();
        last_server_time.as_ref().cloned()
    }

    pub async fn server_time(&self) -> Result<ServerTime> {
        if let Some(cached) = self.cached_server_time() {
            return Ok(cached);
        }

        let time_endpoint = format!("{}/current_time", ChannelState::server_root_url());
        log::info!("Sending server time request to {}", &time_endpoint);
        let res = self
            .base_client
            .http_client()
            .get(&time_endpoint)
            .send()
            .await?;

        if !res.status().is_success() {
            self.observe_iap_challenge(&res);
        }

        match res.status() {
            StatusCode::OK => {
                let time_response: TimeResponse = res.json().await?;
                log::info!(
                    "Received current time from server: {:?}",
                    &time_response.current_time
                );
                let server_time = ServerTime {
                    time_at_fetch: time_response.current_time,
                    fetched_at: Instant::now(),
                };
                let res = Ok(server_time.clone());
                self.set_server_time(server_time);

                res
            }
            _ => {
                let payload: ClientError = res.json().await?;
                Err(anyhow!(payload).context("fetching time from server failed"))
            }
        }
    }

    /// Fetches updated Warp Channel Versions from Warp Server. If it is the first such request of
    /// the current calendar day, first attempts to call the '/client_version/daily'. If that call
    /// fails or if it not the first request of the calendar day, returns the result of a call to
    /// `/client_version'. The caller can specify whether or not changelog information should be
    /// included in the response based on whether or not it will be used.
    pub async fn fetch_channel_versions(
        &self,
        include_changelogs: bool,
        is_daily: bool,
    ) -> Result<ChannelVersions> {
        let mut url = Url::parse(&ChannelState::server_root_url())
            .expect("Should not fail to parse server root URL");
        if is_daily {
            url.set_path("/client_version/daily");
        } else {
            url.set_path("/client_version");
        }
        url.query_pairs_mut()
            .append_pair("include_changelogs", &include_changelogs.to_string());

        if include_changelogs {
            log::info!("Fetching channel versions and changelogs from Warp server");
        } else {
            log::info!("Fetching channel versions (without changelogs) from Warp server");
        }

        let mut request_builder = self
            .base_client
            .http_client()
            .get(url.as_str())
            .timeout(FETCH_CHANNEL_VERSIONS_TIMEOUT)
            .header(EXPERIMENT_ID_HEADER, self.anonymous_id());

        // Authorization for /client_version is optional. Attach authorization header if an access
        // token is present. First, try to get a valid token. If our cached one is expired, try to
        // refresh. Failing that, send the expired token.
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .ok()
            .and_then(|token| token.bearer_token())
            .or_else(|| self.access_token_ignoring_validity());
        if let Some(token_str) = auth_token {
            request_builder = request_builder.bearer_auth(token_str);
        }

        let response = request_builder.send().await?;
        if !response.status().is_success() {
            self.observe_iap_challenge(&response);
        }
        let versions: ChannelVersions = response.json().await?;
        log::info!("Received channel versions from Warp server: {versions}");
        Ok(versions)
    }
}

/// A singleton entity that provides access to the global [`ServerApi`] instance,
/// or any of its implemented trait objects.
pub struct ServerApiProvider {
    server_api: Arc<ServerApi>,
    auth_client: Arc<dyn AuthClient>,
}

impl ServerApiProvider {
    /// Constructs a new ServerApiProvider.
    #[cfg_attr(target_family = "wasm", allow(unused_variables))]
    pub fn new(
        auth_state: Arc<AuthState>,
        agent_source: Option<ai::AgentSource>,
        iap_state: Option<Arc<IapState>>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let (event_sender, event_receiver) = async_channel::bounded(10);

        let server_api = ServerApi::new(
            auth_state.clone(),
            event_sender,
            agent_source,
            iap_state,
            ctx,
        );

        ctx.spawn_stream_local(
            event_receiver,
            move |_, event, ctx| {
                match event {
                    AuthEvent::UserAccountDisabled => {
                        // We dispatch a global action here because the log out code requires
                        // `server_api`, causing a circular model reference panic when it calls
                        // `ServerApiProvider` to get access.
                        // TODO: We should remove this pattern where `ServerApiProvider` responds
                        // to events; it's prone to these sorts of circular reference issues.
                        ctx.dispatch_global_action("app:log_out", ());
                    }
                    AuthEvent::NeedsReauth => {
                        // AuthManager depends on a reference to ServerApi, so ServerApi can't easily
                        // hold a ref to AuthManager. To get around this, we emit an event on ServerApi
                        // and handle calling the AuthManager here instead.
                        AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                            auth_manager.set_needs_reauth(true, ctx);
                        });
                    }
                    AuthEvent::IapChallengeReceived => {
                        IapManager::handle(ctx)
                            .update(ctx, |manager, ctx| manager.handle_challenge(ctx));
                    }
                    // Re-emit the event for subscribers.
                    // TODO: we probably want a different type for the event emitted to subscribers
                    // from the one that's used for the async channel.
                    _ => ctx.emit(event),
                }
            },
            |_, _| {},
        );
        let server_api = Arc::new(server_api);
        let auth_client = Arc::new(AuthClientImpl::new(server_api.base_client.clone()));
        Self {
            server_api,
            auth_client,
        }
    }

    /// Handles fetching server-side experiments by updating the appropriate app state.
    pub fn handle_experiments_fetched(
        &self,
        experiments: Vec<ServerExperiment>,
        ctx: &mut ModelContext<Self>,
    ) {
        ServerExperiments::handle(ctx).update(ctx, |state, ctx| {
            state.apply_latest_state(experiments, ctx);
        });

        settings_view::handle_experiment_change(ctx);
    }

    /// Constructs a new SeverApiProvider for tests.
    #[cfg(any(test, all(feature = "tui", feature = "test-util")))]
    pub fn new_for_test() -> Self {
        let server_api = Arc::new(ServerApi::new_for_test());
        let auth_client = Arc::new(AuthClientImpl::new(server_api.base_client.clone()));
        Self {
            server_api,
            auth_client,
        }
    }

    /// Returns a handle to the underlying [`ServerApi`] object.
    /// Prefer retrieving a specific trait object related to the methods you're calling.
    pub fn get(&self) -> Arc<ServerApi> {
        self.server_api.clone()
    }

    pub fn get_auth_client(&self) -> Arc<dyn AuthClient> {
        self.auth_client.clone()
    }

    pub fn get_referrals_client(&self) -> Arc<dyn ReferralsClient> {
        self.server_api.clone()
    }

    pub fn get_block_client(&self) -> Arc<dyn BlockClient> {
        self.server_api.clone()
    }

    pub fn get_workspace_client(&self) -> Arc<dyn WorkspaceClient> {
        self.server_api.clone()
    }

    pub fn get_team_client(&self) -> Arc<dyn TeamClient> {
        self.server_api.clone()
    }

    pub fn get_ai_client(&self) -> Arc<dyn AIClient> {
        self.server_api.clone()
    }

    pub fn get_cloud_objects_client(&self) -> Arc<dyn ObjectClient> {
        self.server_api.clone()
    }

    pub fn get_integrations_client(&self) -> Arc<dyn integrations::IntegrationsClient> {
        self.server_api.clone()
    }

    pub fn get_managed_secrets_client(&self) -> Arc<dyn ManagedSecretsClient> {
        self.server_api.clone()
    }

    #[cfg_attr(target_family = "wasm", expect(dead_code))]
    pub fn get_managed_mcp_client(&self) -> Arc<dyn ManagedMcpClient> {
        self.server_api.clone()
    }

    pub fn get_factory_client(&self) -> Arc<dyn FactoryClient> {
        self.server_api.clone()
    }

    /// Returns the shared HTTP client. This client is wired into network logging
    /// and includes standard Warp request headers.
    pub fn get_http_client(&self) -> Arc<http_client::Client> {
        self.server_api.owned_http_client()
    }

    #[cfg_attr(target_family = "wasm", expect(dead_code))]
    pub fn get_harness_support_client(&self) -> Arc<dyn harness_support::HarnessSupportClient> {
        self.server_api.clone()
    }
}

impl Entity for ServerApiProvider {
    type Event = AuthEvent;
}

impl SingletonEntity for ServerApiProvider {}
