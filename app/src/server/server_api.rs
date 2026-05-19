pub mod ai;
pub mod auth;
pub mod block;
pub mod harness_support;
pub mod integrations;
pub mod managed_secrets;
pub mod object;
pub(crate) mod presigned_upload;
pub mod referral;
pub mod team;
pub mod workspace;

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::get_relevant_files::api::{GetRelevantFiles, GetRelevantFilesResponse};
use crate::ai::predict::generate_ai_input_suggestions;
use crate::ai::predict::generate_ai_input_suggestions::GenerateAIInputSuggestionsRequest;
use crate::ai::predict::generate_am_query_suggestions;
use crate::ai::predict::generate_am_query_suggestions::GenerateAMQuerySuggestionsRequest;
use crate::ai::predict::predict_am_queries::{PredictAMQueriesRequest, PredictAMQueriesResponse};
use crate::ai::voice::transcribe::{TranscribeRequest, TranscribeResponse};
use crate::auth::auth_manager::AuthManager;
use crate::auth::auth_state::AuthState;
use crate::auth::UserUid;
use crate::server::graphql::default_request_options;
use crate::server::server_api::presigned_upload::HttpStatusError;
use ai::AIClient;
use auth::{AuthClient, AMBIENT_WORKLOAD_TOKEN_HEADER, CLOUD_AGENT_ID_HEADER};
use base64::prelude::BASE64_URL_SAFE;
use base64::Engine;
use block::BlockClient;
use channel_versions::ChannelVersions;
use futures::StreamExt;
use object::ObjectClient;
use prost::Message;
use referral::ReferralsClient;
use team::TeamClient;
use url::Url;
use warp_core::context_flag::ContextFlag;
use warp_core::errors::{register_error, AnyhowErrorExt, ErrorExt};
use warp_managed_secrets::client::ManagedSecretsClient;
use warpui::{r#async::BoxFuture, ModelContext};
use workspace::WorkspaceClient;

use crate::server::telemetry::TelemetryApi;
use crate::settings::PrivacySettingsSnapshot;
use crate::settings_view;

use crate::ChannelState;

use ::http::header::CONTENT_LENGTH;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, FixedOffset};
use instant::Instant;
use parking_lot::{Mutex, RwLock};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use warp_core::telemetry::TelemetryEvent;
use warpui::Entity;
use warpui::SingletonEntity;

use super::experiments::ServerExperiment;
use super::experiments::ServerExperiments;
use super::graphql::GraphQLError;

pub const FETCH_CHANNEL_VERSIONS_TIMEOUT: std::time::Duration = Duration::from_secs(60);

const EXPERIMENT_ID_HEADER: &str = "X-Warp-Experiment-Id";

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

/// Header used to communicate the source of an agent run (e.g. "CLI", "GITHUB_ACTION").
pub(crate) const AGENT_SOURCE_HEADER: &str = "X-Oz-Api-Source";

#[cfg(feature = "agent_mode_evals")]
pub const EVAL_USER_ID_HEADER: &str = "X-Eval-User-ID";

/// IDs in the staging database that were created specifically for evals.
/// These users have a clean state where they haven't been referred nor have referred anyone (which causes a popup in the client).
/// DO NOT REMOVE OR CHANGE THESE USERS!
///
/// Keep this list in sync with `script/populate_agent_mode_eval_user.sql`
/// in warp-server. Those rows need to exist in the DB so the authz user loader
/// can resolve these IDs during task creation; otherwise the server will 500
/// on every eval request with a nil-deref in `UserIDFromUser`.
#[cfg(feature = "agent_mode_evals")]
const EVAL_USER_IDS: [i32; 11] = [
    2162, 2164, 2165, 2166, 2167, 2168, 2169, 2172, 2173, 2174, 2175,
];

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
    async fn from_stream_error(stream_type: &'static str, err: reqwest_eventsource::Error) -> Self {
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

    /// Returns whether or not the error can be retried.
    pub fn is_retryable(&self) -> bool {
        // Don't retry client errors, except for timeouts and quota limits.
        fn is_retryable_status(status: http::StatusCode) -> bool {
            !status.is_client_error()
                || status == http::StatusCode::REQUEST_TIMEOUT
                || status == http::StatusCode::TOO_MANY_REQUESTS
        }

        match self {
            AIApiError::ErrorStatus(status, _) => is_retryable_status(*status),
            AIApiError::Transport(e) => {
                if let Some(status) = e.status() {
                    return is_retryable_status(status);
                }
                true
            }
            // By default, retry on error.
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
            AIApiError::ErrorStatus(_, _) => self.is_retryable(),
            AIApiError::QuotaLimit { .. }
            | AIApiError::ServerOverloaded
            | AIApiError::NoContextFound => false,
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

cfg_if::cfg_if! {
    if #[cfg(target_family = "wasm")] {
        // The WASM version of this type has no bound on `Send`, which is not implemented on
        // `wasm_bindgen::JsValue`, which is ultimately used in reqwest_eventsource::Error. Furthermore,
        // `Send` is an unnecessary bound when targeting wasm because the browser is single-threaded (and
        // we don't leverage WebWorkers for async execution in WoW).
        pub type AIOutputStream<T> = futures::stream::LocalBoxStream<'static, Result<T, Arc<AIApiError>>>;
    } else {
        pub type AIOutputStream<T> = futures::stream::BoxStream<'static, Result<T, Arc<AIApiError>>>;
    }
}

/// An event related to the server API itself (and not a particular API call).
/// Most errors should be handled in callbacks to individual APIs, rather than sent over the
/// server API channel.
#[derive(Clone)]
pub enum ServerApiEvent {
    /// We made a staging API call that was blocked, which may indicate a firewall misconfiguration.
    StagingAccessBlocked,
    /// The user's access token was invalid, so they need to reauth before they can make
    /// requests to warp-server.
    NeedsReauth,
    /// The user's account has been disabled.
    UserAccountDisabled,
    /// The current bearer token was refreshed.
    AccessTokenRefreshed {
        #[cfg_attr(target_family = "wasm", allow(dead_code))]
        token: String,
    },
}

impl fmt::Debug for ServerApiEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StagingAccessBlocked => f.write_str("StagingAccessBlocked"),
            Self::NeedsReauth => f.write_str("NeedsReauth"),
            Self::UserAccountDisabled => f.write_str("UserAccountDisabled"),
            Self::AccessTokenRefreshed { .. } => f
                .debug_struct("AccessTokenRefreshed")
                .field("token", &"<redacted>")
                .finish(),
        }
    }
}

/// An API wrapper struct with methods to requests to warp-server.
///
/// Prefer NOT adding new methods directly on this struct; instead, add to one of the existing
/// client trait objects, or create your own. This helps keep `ServerApi` from being overloaded
/// with disparate types of calls, and allows you to mock methods in tests.
pub struct ServerApi {
    client: Arc<http_client::Client>,
    auth_state: Arc<AuthState>,
    event_sender: async_channel::Sender<ServerApiEvent>,
    // TODO(jeff): Make `TelemetryApi` another type of client, and move it off `ServerApi`.
    telemetry_api: TelemetryApi,
    last_server_time: Arc<Mutex<Option<ServerTime>>>,
    // We technically use OAuth2 for headless device authentication.
    oauth_client: self::auth::OAuth2Client,
    /// Cached ambient workload token for requests from ambient agents.
    ambient_workload_token: Arc<Mutex<Option<warp_isolation_platform::WorkloadToken>>>,
    /// The ambient agent task ID for requests from cloud agents.
    ambient_agent_task_id: Arc<RwLock<Option<AmbientAgentTaskId>>>,
    /// The source of agent runs (e.g. CLI, GitHub Action). Set once at startup and immutable.
    agent_source: Option<ai::AgentSource>,

    #[cfg(feature = "agent_mode_evals")]
    eval_user_id: Option<i32>,
}

impl ServerApi {
    fn new(
        auth_state: Arc<AuthState>,
        event_sender: async_channel::Sender<ServerApiEvent>,
        agent_source: Option<ai::AgentSource>,
    ) -> Self {
        let client = Arc::new(http_client::Client::new());
        Self::new_with_parts(client, auth_state, event_sender, agent_source)
    }

    fn new_with_parts(
        client: Arc<http_client::Client>,
        auth_state: Arc<AuthState>,
        event_sender: async_channel::Sender<ServerApiEvent>,
        agent_source: Option<ai::AgentSource>,
    ) -> Self {
        // We generate a random user ID for evals so we can run evals in parallel.
        #[cfg(feature = "agent_mode_evals")]
        let eval_user_id = {
            use rand::Rng;
            Some(EVAL_USER_IDS[rand::thread_rng().gen_range(0..EVAL_USER_IDS.len())])
        };

        let oauth_client = Self::create_oauth_client();

        Self {
            client,
            auth_state,
            event_sender,
            telemetry_api: TelemetryApi::new(),
            last_server_time: Arc::new(Mutex::new(None)),
            oauth_client,
            ambient_workload_token: Arc::new(Mutex::new(None)),
            ambient_agent_task_id: Arc::new(RwLock::new(None)),
            agent_source,
            #[cfg(feature = "agent_mode_evals")]
            eval_user_id,
        }
    }

    #[cfg(test)]
    fn new_for_test() -> Self {
        let (tx, _) = async_channel::unbounded();
        let auth_state = Arc::new(AuthState::new_for_test());
        let client = Arc::new(http_client::Client::new_for_test());

        Self::new_with_parts(client, auth_state, tx, None)
    }

    #[cfg(test)]
    fn new_for_test_with_bearer_token(
        bearer_token: Option<String>,
        event_sender: async_channel::Sender<ServerApiEvent>,
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
        )
    }

    pub fn allowed_to_refresh_token(&self) -> bool {
        self.auth_state
            .credentials()
            .is_none_or(|credentials| !credentials.is_externally_managed())
    }

    fn access_token_ignoring_validity(&self) -> Option<String> {
        self.auth_state.get_access_token_ignoring_validity()
    }

    pub(super) fn anonymous_id(&self) -> String {
        self.auth_state.anonymous_id()
    }

    fn user_id(&self) -> Option<UserUid> {
        self.auth_state.user_id()
    }

    /// Sets the ambient agent task ID to be sent with all subsequent requests.
    pub fn set_ambient_agent_task_id(&self, task_id: Option<AmbientAgentTaskId>) {
        *self.ambient_agent_task_id.write() = task_id;
    }

    /// Returns ambient agent headers to attach to requests.
    async fn ambient_agent_headers(&self) -> Result<Vec<(&'static str, String)>> {
        let workload_token = self
            .get_or_create_ambient_workload_token()
            .await
            .context("Failed to get ambient workload token")?;

        let task_id = self
            .ambient_agent_task_id
            .read()
            .as_ref()
            .map(|id| id.to_string());

        let agent_source = self.agent_source.as_ref().map(|s| s.as_str().to_string());

        Ok(workload_token
            .map(|token| (AMBIENT_WORKLOAD_TOKEN_HEADER, token))
            .into_iter()
            .chain(task_id.map(|id| (CLOUD_AGENT_ID_HEADER, id)))
            .chain(agent_source.map(|s| (AGENT_SOURCE_HEADER, s)))
            .collect())
    }

    async fn ambient_agent_headers_for_task(
        &self,
        task_id: &AmbientAgentTaskId,
    ) -> Result<Vec<(&'static str, String)>> {
        let mut headers = self.ambient_agent_headers().await?;
        headers.retain(|(name, _)| *name != CLOUD_AGENT_ID_HEADER);
        headers.push((CLOUD_AGENT_ID_HEADER, task_id.to_string()));
        Ok(headers)
    }

    fn create_oauth_client() -> self::auth::OAuth2Client {
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

    pub fn send_graphql_request<'a, QF, O: warp_graphql::client::Operation<QF> + Send + 'a>(
        &'a self,
        operation: O,
        timeout: Option<Duration>,
    ) -> BoxFuture<'a, Result<QF>> {
        let client = self.client.clone();
        let event_sender = self.event_sender.clone();

        #[cfg(feature = "agent_mode_evals")]
        let headers = if let Some(eval_user_id) = self.eval_user_id {
            std::collections::HashMap::from([(
                EVAL_USER_ID_HEADER.to_string(),
                eval_user_id.to_string(),
            )])
        } else {
            Default::default()
        };

        Box::pin(async move {
            let operation_name = operation.operation_name().map(Cow::into_owned);
            let auth_token = self
                .access_token()
                .await
                .context("Failed to get access token for GraphQL request")?;

            #[cfg(feature = "agent_mode_evals")]
            let mut headers = headers;
            #[cfg(not(feature = "agent_mode_evals"))]
            let mut headers = std::collections::HashMap::new();

            for (name, value) in self.ambient_agent_headers().await? {
                headers.insert(name.to_string(), value);
            }

            let options = warp_graphql::client::RequestOptions {
                auth_token: auth_token.bearer_token(),
                timeout,
                headers,
                ..default_request_options()
            };

            let response = match operation.send_request(client, options).await {
                Ok(response) => response,
                Err(GraphQLError::StagingAccessBlocked) => {
                    let _ = event_sender.try_send(ServerApiEvent::StagingAccessBlocked);
                    anyhow::bail!(GraphQLError::StagingAccessBlocked)
                }
                Err(err) => {
                    if !self.allowed_to_refresh_token() && Self::is_graphql_auth_rejection(&err) {
                        anyhow::bail!("server rejected authentication credentials");
                    }
                    anyhow::bail!(err)
                }
            };

            if let Some(errors) = response.errors.as_ref() {
                crate::safe_error!(
                    safe: ("graphql response for {:?} had errors", operation_name),
                    full: ("graphql response for {:?} had errors {:?}", operation_name, errors)
                );

                // "User not in context: Not found" comes from warp-server as an error when attempting
                // to get a required user for some gql field. If we see that, since we have already
                // successfully refreshed the user's access token earlier in this function, we know
                // that this error is the result of the user's account being disabled/deleted.
                if errors
                    .iter()
                    .any(|error| error.message.contains("User not in context: Not found"))
                {
                    if self.allowed_to_refresh_token() {
                        log::error!("GraphQL request failed due to unauthenticated user");
                        let _ = event_sender.try_send(ServerApiEvent::UserAccountDisabled);
                    } else {
                        anyhow::bail!("server rejected authentication credentials");
                    }
                }
            }

            response.data.ok_or_else(|| {
                let operation_label = operation_name
                    .as_deref()
                    .unwrap_or("unknown GraphQL operation");
                let error_messages = response
                    .errors
                    .as_ref()
                    .map(|errors| {
                        errors
                            .iter()
                            .filter_map(|error| {
                                let message = error.message.trim();
                                (!message.is_empty()).then(|| message.to_string())
                            })
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
                    .filter(|messages| !messages.is_empty());

                match error_messages {
                    Some(messages) => {
                        anyhow!("missing response data for {operation_label}: {messages}")
                    }
                    None => anyhow!("missing response data for {operation_label}"),
                }
            })
        })
    }

    fn is_graphql_auth_rejection(err: &GraphQLError) -> bool {
        match err {
            GraphQLError::HttpError { status, .. } => {
                *status == StatusCode::UNAUTHORIZED || *status == StatusCode::FORBIDDEN
            }
            GraphQLError::RequestError(_)
            | GraphQLError::StagingAccessBlocked
            | GraphQLError::ResponseError(_) => false,
        }
    }

    /// Sends a GET request to a public API endpoint.
    ///
    /// # Arguments
    /// * `path` - Endpoint path relative to `/api/v1` (e.g., "agent/tasks/{task_id}")
    async fn get_public_api<R>(&self, path: &str) -> Result<R>
    where
        R: serde::de::DeserializeOwned,
    {
        let response = self.get_public_api_response(path).await?;
        let url = response.url().clone();
        response
            .json::<R>()
            .await
            .with_context(|| format!("Failed to deserialize response from {url}"))
    }

    /// Sends a GET request to a public API endpoint and returns the raw response on success.
    ///
    /// Unlike [`get_public_api`], this does not attempt JSON deserialization on the
    /// response body, allowing the caller to decode it however they need.
    async fn get_public_api_response(&self, path: &str) -> Result<http_client::Response> {
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for API request")?;

        let url = format!("{}/api/v1/{}", ChannelState::server_root_url(), path);

        let mut request = self.client.get(&url);
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
            // Put `HttpStatusError` in the error chain so shared retry classifiers
            // (`is_transient_http_error`) can distinguish transient 5xx / 408 / 429
            // from permanent 4xx without string-matching the Display output.
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let status_err = HttpStatusError {
                status: status.as_u16(),
                body: body.clone(),
            };
            match serde_json::from_str::<ClientError>(&body) {
                Ok(error_response) => {
                    Err(anyhow::Error::new(status_err).context(error_response.error))
                }
                Err(_) => Err(anyhow::Error::new(status_err)
                    .context(format!("API request failed with status {status}"))),
            }
        }
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

        let mut request = self.client.get(&url);
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }

        for (name, value) in self.ambient_agent_headers().await? {
            request = request.header(name, value);
        }

        Ok(request.eventsource())
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

        let mut request = self.client.get(&url);
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }

        for (name, value) in self.ambient_agent_headers_for_task(task_id).await? {
            request = request.header(name, value);
        }

        Ok(request.eventsource())
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

        let mut request = self.client.post(&url).json(body);
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
        if is_at_capacity {
            if let Ok(capacity_error) =
                serde_json::from_str::<CloudAgentCapacityError>(&response_text)
            {
                return capacity_error.into();
            }
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

    /// Sends a POST request to a public API endpoint that returns no response body.
    async fn post_public_api_unit<B>(&self, path: &str, body: &B) -> Result<()>
    where
        B: Serialize,
    {
        self.post_public_api_response(path, body).await?;
        Ok(())
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

        let mut request = self.client.patch(&url).json(body);
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
                let mut request = self.client.post(&url);
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
                    log::error!("Failed to send POST request to /client/login: {err:?}");
                }
            }
            Err(err) => {
                log::error!("Could not retrieve access token for notifying user login: {err:?}");
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

        let request_builder = self.client.post(format!(
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

        let request_builder = self.client.post(format!(
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

        let request_builder = self.client.post(url);
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
        let request_builder = self.client.post(format!(
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
            .client
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

    pub async fn generate_multi_agent_output(
        &self,
        request: &warp_multi_agent_api::Request,
    ) -> std::result::Result<AIOutputStream<warp_multi_agent_api::ResponseEvent>, Arc<AIApiError>>
    {
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .map_err(Into::into)
            .map_err(Arc::new)?;

        let is_passive = request.input.as_ref().is_some_and(|input| {
            matches!(
                input.r#type,
                Some(warp_multi_agent_api::request::input::Type::GeneratePassiveSuggestions(_))
            )
        });
        let is_evals = cfg!(feature = "agent_mode_evals");
        let url = format!(
            "{}/{}/{}",
            ChannelState::server_root_url(),
            if is_evals { "agent-mode-evals" } else { "ai" },
            if is_passive {
                "passive-suggestions"
            } else {
                "multi-agent"
            }
        );

        let ambient_workload_token = self
            .get_or_create_ambient_workload_token()
            .await
            .map_err(Into::into)
            .map_err(Arc::new)?;

        let mut request_builder = self
            .client
            .post(url)
            .proto(request)
            .prevent_sleep("Agent Mode request in-progress");
        if let Some(token) = auth_token.as_bearer_token() {
            request_builder = request_builder.bearer_auth(token);
        }

        if let Some(token) = ambient_workload_token {
            request_builder = request_builder.header(AMBIENT_WORKLOAD_TOKEN_HEADER, token);
        }

        cfg_if::cfg_if! {
            if #[cfg(feature = "agent_mode_evals")] {
                let mut request = request_builder;
                if let Some(eval_user_id) = self.eval_user_id {
                    request = request.header(EVAL_USER_ID_HEADER, eval_user_id.to_string());
                }
            } else {
                let request = request_builder;
            }
        }

        let output_stream = request.eventsource().filter_map(|event| async {
            let result = match event {
                Ok(reqwest_eventsource::Event::Message(message_event)) => {
                    match BASE64_URL_SAFE.decode(message_event.data.trim_matches('"')) {
                        Ok(decoded_data) => {
                            let action = warp_multi_agent_api::ResponseEvent::decode(
                                decoded_data.as_slice(),
                            );
                            Some(action.map_err(|e| AIApiError::Other(anyhow::Error::from(e))))
                        }
                        Err(e) => Some(Err(AIApiError::Other(anyhow::Error::from(e)))),
                    }
                }
                Ok(reqwest_eventsource::Event::Open) => None,
                Err(err) => Some(Err(AIApiError::from_stream_error(
                    "GenerateMultiAgentOutput",
                    err,
                )
                .await)),
            }
            // Wrap errors in an Arc so that they're cloneable by downstream event
            // handlers.
            .map(|item| item.map_err(Arc::new));
            result
        });

        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                Ok(output_stream.boxed_local())
            } else {
                Ok(output_stream.boxed())
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

    /// Returns the inner `http_client::Client` used by the `ServerApi`. Callers can use this long-lived
    /// client to make requests without having to create a new client.
    pub fn http_client(&self) -> &http_client::Client {
        &self.client
    }

    pub async fn server_time(&self) -> Result<ServerTime> {
        if let Some(cached) = self.cached_server_time() {
            return Ok(cached);
        }

        let time_endpoint = format!("{}/current_time", ChannelState::server_root_url());
        log::info!("Sending server time request to {}", &time_endpoint);
        let res = self.client.get(&time_endpoint).send().await?;

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
            .client
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
        let versions: ChannelVersions = response.json().await?;
        log::info!("Received channel versions from Warp server: {versions}");
        Ok(versions)
    }
}

/// A singleton entity that provides access to the global [`ServerApi`] instance,
/// or any of its implemented trait objects.
pub struct ServerApiProvider {
    server_api: Arc<ServerApi>,
}

impl ServerApiProvider {
    /// Constructs a new ServerApiProvider.
    pub fn new(
        auth_state: Arc<AuthState>,
        agent_source: Option<ai::AgentSource>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let (event_sender, event_receiver) = async_channel::bounded(10);
        let mut server_api = ServerApi::new(auth_state.clone(), event_sender, agent_source);

        if ContextFlag::NetworkLogConsole.is_enabled() {
            super::network_logging::init(
                [
                    Arc::get_mut(&mut server_api.client)
                        .expect("guaranteed there is only one copy of client"),
                    &mut server_api.telemetry_api.client,
                ],
                ctx,
            );
        }

        ctx.spawn_stream_local(
            event_receiver,
            move |_, event, ctx| {
                match event {
                    ServerApiEvent::UserAccountDisabled => {
                        // We dispatch a global action here because the log out code requires
                        // `server_api`, causing a circular model reference panic when it calls
                        // `ServerApiProvider` to get access.
                        // TODO: We should remove this pattern where `ServerApiProvider` responds
                        // to events; it's prone to these sorts of circular reference issues.
                        ctx.dispatch_global_action("app:log_out", ());
                    }
                    ServerApiEvent::NeedsReauth => {
                        // AuthManager depends on a reference to ServerApi, so ServerApi can't easily
                        // hold a ref to AuthManager. To get around this, we emit an event on ServerApi
                        // and handle calling the AuthManager here instead.
                        AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                            auth_manager.set_needs_reauth(true, ctx);
                        });
                    }
                    // Re-emit the event for subscribers.
                    // TODO: we probably want a different type for the event emitted to subscribers
                    // from the one that's used for the async channel.
                    _ => ctx.emit(event),
                }
            },
            |_, _| {},
        );
        Self {
            server_api: Arc::new(server_api),
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
    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self {
            server_api: Arc::new(ServerApi::new_for_test()),
        }
    }

    /// Returns a handle to the underlying [`ServerApi`] object.
    /// Prefer retrieving a specific trait object related to the methods you're calling.
    pub fn get(&self) -> Arc<ServerApi> {
        self.server_api.clone()
    }

    pub fn get_auth_client(&self) -> Arc<dyn AuthClient> {
        self.server_api.clone()
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

    /// Returns the shared HTTP client. This client is wired into network logging
    /// and includes standard Warp request headers.
    pub fn get_http_client(&self) -> Arc<http_client::Client> {
        self.server_api.client.clone()
    }

    #[cfg_attr(target_family = "wasm", expect(dead_code))]
    pub fn get_harness_support_client(&self) -> Arc<dyn harness_support::HarnessSupportClient> {
        self.server_api.clone()
    }
}

impl Entity for ServerApiProvider {
    type Event = ServerApiEvent;
}

impl SingletonEntity for ServerApiProvider {}

#[cfg(test)]
mod tests {
    use super::*;

    use cynic::{GraphQlError, GraphQlResponse};
    use futures::executor::block_on;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeGraphqlOperation {
        expected_auth_token: Option<String>,
        send_count: Arc<AtomicUsize>,
        result: FakeGraphqlResult,
    }

    enum FakeGraphqlResult {
        Success,
        Rejected(StatusCode),
        ResponseErrors(Vec<String>),
    }

    impl FakeGraphqlOperation {
        fn successful(expected_auth_token: Option<&str>, send_count: Arc<AtomicUsize>) -> Self {
            Self {
                expected_auth_token: expected_auth_token.map(ToOwned::to_owned),
                send_count,
                result: FakeGraphqlResult::Success,
            }
        }

        fn rejected(
            expected_auth_token: Option<&str>,
            send_count: Arc<AtomicUsize>,
            status: StatusCode,
        ) -> Self {
            Self {
                expected_auth_token: expected_auth_token.map(ToOwned::to_owned),
                send_count,
                result: FakeGraphqlResult::Rejected(status),
            }
        }

        fn response_errors(
            expected_auth_token: Option<&str>,
            send_count: Arc<AtomicUsize>,
            messages: Vec<String>,
        ) -> Self {
            Self {
                expected_auth_token: expected_auth_token.map(ToOwned::to_owned),
                send_count,
                result: FakeGraphqlResult::ResponseErrors(messages),
            }
        }
    }

    impl warp_graphql::client::Operation<()> for FakeGraphqlOperation {
        fn operation_name(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed("FakeGraphqlOperation"))
        }

        fn send_request(
            self,
            _client: Arc<http_client::Client>,
            options: warp_graphql::client::RequestOptions,
        ) -> Pin<
            Box<
                dyn Future<Output = std::result::Result<GraphQlResponse<()>, GraphQLError>>
                    + Send
                    + 'static,
            >,
        >
        where
            Self: Sized,
        {
            Box::pin(async move {
                assert_eq!(options.auth_token, self.expected_auth_token);
                self.send_count.fetch_add(1, Ordering::SeqCst);
                match self.result {
                    FakeGraphqlResult::Success => Ok(GraphQlResponse {
                        data: Some(()),
                        errors: None,
                    }),
                    FakeGraphqlResult::Rejected(status) => Err(GraphQLError::HttpError {
                        status,
                        body: "redacted auth rejection".to_string(),
                    }),
                    FakeGraphqlResult::ResponseErrors(messages) => Ok(GraphQlResponse {
                        data: None,
                        errors: Some(
                            messages
                                .into_iter()
                                .map(|message| GraphQlError::new(message, None, None, None))
                                .collect(),
                        ),
                    }),
                }
            })
        }
    }

    fn has_error_message(error: &anyhow::Error, expected: &str) -> bool {
        error.chain().any(|cause| cause.to_string() == expected)
    }

    #[test]
    fn send_graphql_request_refresh_enabled_uses_auth_state() {
        let server_api = ServerApi::new_for_test();
        let send_count = Arc::new(AtomicUsize::new(0));

        block_on(server_api.send_graphql_request(
            FakeGraphqlOperation::successful(None, send_count.clone()),
            None,
        ))
        .unwrap();

        assert!(server_api.allowed_to_refresh_token());
        assert_eq!(send_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn send_graphql_request_refresh_disabled_uses_provided_bearer_token() {
        let (event_sender, _) = async_channel::unbounded();
        let server_api = ServerApi::new_for_test_with_bearer_token(
            Some("daemon-token".to_string()),
            event_sender,
        );
        let send_count = Arc::new(AtomicUsize::new(0));

        block_on(server_api.send_graphql_request(
            FakeGraphqlOperation::successful(Some("daemon-token"), send_count.clone()),
            None,
        ))
        .unwrap();

        assert!(!server_api.allowed_to_refresh_token());
        assert_eq!(send_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn send_graphql_request_refresh_disabled_missing_token_returns_auth_error() {
        let (event_sender, event_receiver) = async_channel::unbounded();
        let server_api = ServerApi::new_for_test_with_bearer_token(None, event_sender);
        let send_count = Arc::new(AtomicUsize::new(0));

        let error = block_on(server_api.send_graphql_request(
            FakeGraphqlOperation::successful(Some("unused-token"), send_count.clone()),
            None,
        ))
        .unwrap_err();

        assert!(has_error_message(
            &error,
            "missing authentication credentials"
        ));
        assert_eq!(send_count.load(Ordering::SeqCst), 0);
        assert!(event_receiver.try_recv().is_err());
    }

    #[test]
    fn send_graphql_request_refresh_disabled_auth_rejection_is_credentials_rejected() {
        let (event_sender, event_receiver) = async_channel::unbounded();
        let server_api = ServerApi::new_for_test_with_bearer_token(
            Some("daemon-token".to_string()),
            event_sender,
        );
        let send_count = Arc::new(AtomicUsize::new(0));

        let error = block_on(server_api.send_graphql_request(
            FakeGraphqlOperation::rejected(
                Some("daemon-token"),
                send_count.clone(),
                StatusCode::UNAUTHORIZED,
            ),
            None,
        ))
        .unwrap_err();

        assert!(has_error_message(
            &error,
            "server rejected authentication credentials"
        ));
        assert_eq!(send_count.load(Ordering::SeqCst), 1);
        assert!(event_receiver.try_recv().is_err());
    }

    #[test]
    fn send_graphql_request_refresh_disabled_user_not_in_context_is_credentials_rejected() {
        let (event_sender, event_receiver) = async_channel::unbounded();
        let server_api = ServerApi::new_for_test_with_bearer_token(
            Some("daemon-token".to_string()),
            event_sender,
        );
        let send_count = Arc::new(AtomicUsize::new(0));

        let error = block_on(server_api.send_graphql_request(
            FakeGraphqlOperation::response_errors(
                Some("daemon-token"),
                send_count.clone(),
                vec!["User not in context: Not found".to_string()],
            ),
            None,
        ))
        .unwrap_err();

        assert!(has_error_message(
            &error,
            "server rejected authentication credentials"
        ));
        assert_eq!(send_count.load(Ordering::SeqCst), 1);
        assert!(event_receiver.try_recv().is_err());
    }
}
