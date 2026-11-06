//! Client for the pre-warmed sandbox endpoints a standby sandbox uses to fetch its
//! configuration, report preparation state, and prove it is still alive.
//!
//! Every endpoint requires a bearer credential and Namespace workload identity.
// Pre-warmed sandboxes only run in a Namespace container, never in a WASM build.
#![cfg_attr(target_family = "wasm", expect(dead_code))]

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cynic::MutationBuilder;
use futures::{SinkExt as _, StreamExt as _};
use http::HeaderValue;
use serde::{Deserialize, Serialize};
use warp_graphql::client::get_request_context;
use warp_graphql::mutations::prewarmed_sandbox_config::{
    PrewarmedSandboxConfig, PrewarmedSandboxConfigInput, PrewarmedSandboxConfigResult,
    PrewarmedSandboxConfigVariables, PrewarmedSandboxConfiguration as GraphqlConfiguration,
};
use warp_graphql::queries::execution_config::{
    CloudProviderClientConfigs, ExecutionConfiguration, ExecutionRepository, SourceRepo,
};
use warp_managed_secrets::{ManagedSecretValue, convert_task_secrets};
pub use warp_server_client::base_client::PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER;
use warpui::r#async::FutureExt as _;
use websocket::WebsocketMessage as _;

use super::ServerApi;
use super::ai::{GitCredential, into_git_credential};
use super::presigned_upload::HttpStatusError;
use crate::ChannelState;

/// Bounds each complete request, including credential lookup and response decoding.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

pub fn is_prewarmed_sandbox_unavailable(error: &anyhow::Error) -> bool {
    let gone = http::StatusCode::GONE.as_u16();
    #[cfg(not(target_family = "wasm"))]
    if websocket::connect_error_http_response(error)
        .is_some_and(|response| response.status().as_u16() == gone)
    {
        return true;
    }
    error.chain().any(|cause| {
        cause
            .downcast_ref::<HttpStatusError>()
            .is_some_and(|error| error.status == gone)
    })
}

/// Converts an HTTP endpoint URL to its WebSocket form for the given configuration generation.
fn control_socket_url(http_url: &str, generation: &str) -> Result<String> {
    let mut url = url::Url::parse(http_url).context("Invalid pre-warmed sandbox WebSocket URL")?;
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        other => bail!("Unsupported pre-warmed sandbox WebSocket scheme {other}"),
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow!("Invalid pre-warmed sandbox WebSocket scheme"))?;
    url.query_pairs_mut().append_pair("generation", generation);
    Ok(url.to_string())
}

#[cfg(not(target_family = "wasm"))]
fn control_request(
    url: &str,
    access_token: &str,
    workload_token: &str,
) -> Result<websocket::tungstenite::handshake::client::Request> {
    use websocket::IntoClientRequest as _;
    let mut request = url
        .into_client_request()
        .context("Invalid sandbox control URL")?;
    for (name, value) in [
        (
            http::header::AUTHORIZATION.as_str(),
            format!("Bearer {access_token}"),
        ),
        (
            PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER,
            workload_token.to_owned(),
        ),
    ] {
        let mut value =
            HeaderValue::from_str(&value).context("Invalid sandbox control credential header")?;
        value.set_sensitive(true);
        request
            .headers_mut()
            .insert(http::HeaderName::from_bytes(name.as_bytes())?, value);
    }
    Ok(request)
}

/// Defines setup that does not need a task.
#[derive(Clone)]
pub struct PrewarmedSandboxPreparation {
    pub harness: String,
    /// Defines the repository checkouts that preparation must create.
    pub repositories: Vec<ExecutionRepository>,
    /// Keeps repositories for later on-demand clones without cloning them during setup.
    pub deferred_repositories: Vec<SourceRepo>,
    pub setup_commands: Vec<String>,
    /// Uses the agent config file format so normal runs and standby share MCP validation.
    pub mcp_servers: serde_json::Map<String, serde_json::Value>,
    pub skills: Vec<String>,
    pub factory_skill_dirs: Vec<String>,
    pub computer_use_enabled: bool,
    pub providers: Option<CloudProviderClientConfigs>,
}

// Debug output omits values because setup commands and MCP settings can contain secrets.
impl fmt::Debug for PrewarmedSandboxPreparation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrewarmedSandboxPreparation")
            .field("harness", &self.harness)
            .field("repository_count", &self.repositories.len())
            .field(
                "deferred_repository_count",
                &self.deferred_repositories.len(),
            )
            .field("setup_command_count", &self.setup_commands.len())
            .field("mcp_server_count", &self.mcp_servers.len())
            .field("skill_count", &self.skills.len())
            .field("factory_skill_dir_count", &self.factory_skill_dirs.len())
            .field("computer_use_enabled", &self.computer_use_enabled)
            .finish()
    }
}

/// Supplies a standby configuration whose secrets must stay out of logs and saved app state.
#[derive(Clone)]
pub struct PrewarmedSandboxConfiguration {
    /// Identifies the configuration version that status reports and heartbeats must match.
    pub generation: String,
    pub preparation: PrewarmedSandboxPreparation,
    /// Supplies terminal variables that can include authenticated Factory clone URLs.
    pub environment_variables: HashMap<String, String>,
    /// Supplies temporary credentials for the sandbox's Git credential files.
    pub git_credentials: Vec<GitCredential>,
    /// Supplies secret values for the terminal environment and MCP configuration.
    pub managed_secrets: HashMap<String, ManagedSecretValue>,
    /// Sets the server deadline that the standby process must exit before.
    pub expires_at: DateTime<Utc>,
}

impl TryFrom<GraphqlConfiguration> for PrewarmedSandboxConfiguration {
    type Error = anyhow::Error;

    fn try_from(config: GraphqlConfiguration) -> Result<Self> {
        let preparation = config.spec;
        Ok(Self {
            generation: config.fingerprint,
            preparation: PrewarmedSandboxPreparation {
                harness: preparation.harness,
                repositories: preparation.repositories,
                deferred_repositories: preparation.deferred_repositories,
                setup_commands: preparation.setup_commands,
                mcp_servers: serde_json::from_str(&preparation.mcp_servers_json)
                    .map_err(|_| anyhow!("Invalid pre-warmed sandbox MCP server configuration"))?,
                skills: preparation.skills,
                factory_skill_dirs: preparation.factory_skill_dirs,
                computer_use_enabled: preparation.computer_use_enabled,
                providers: preparation.providers,
            },
            environment_variables: config
                .environment_variables
                .into_iter()
                .map(|entry| (entry.name, entry.value))
                .collect(),
            git_credentials: config
                .git_credentials
                .into_iter()
                .map(into_git_credential)
                .collect(),
            managed_secrets: convert_task_secrets(
                config
                    .managed_secrets
                    .into_iter()
                    .map(|entry| (entry.name, entry.value))
                    .collect(),
            )
            .context("Invalid pre-warmed sandbox managed secrets")?,
            expires_at: config.expires_at.utc(),
        })
    }
}

impl fmt::Debug for PrewarmedSandboxConfiguration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrewarmedSandboxConfiguration")
            .field("generation", &self.generation)
            .field("preparation", &self.preparation)
            .field(
                "environment_variable_count",
                &self.environment_variables.len(),
            )
            .field("git_credential_count", &self.git_credentials.len())
            .field("managed_secret_count", &self.managed_secrets.len())
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Restricts standby status reports to the two preparation outcomes the server accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrewarmedSandboxState {
    Ready,
    Failed,
}

impl PrewarmedSandboxState {
    fn as_wire_value(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

/// Reports a preparation outcome for a specific configuration generation.
#[derive(Debug, Serialize)]
struct PrewarmedSandboxStatusInput {
    /// The server's name for the configuration generation.
    fingerprint: String,
    state: &'static str,
    /// Must match the fingerprint for a ready report.
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest_hash: Option<String>,
}

/// Messages received from the pre-warmed sandbox control socket.
///
/// The server commits a run to the sandbox before sending its assignment, so the sandbox has no
/// way to decline it. The assignment is sent again on every connection while the run is active.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PrewarmedSandboxControlEvent {
    Assignment {
        generation: String,
        execution_config: Box<ExecutionConfiguration>,
        runtime_env: HashMap<String, String>,
        api_key: String,
        expires_at: DateTime<Utc>,
    },
    /// Precedes the server closing the socket because the sandbox will never serve a run.
    Retired { reason: Option<String> },
    /// A message type this client does not know, which it ignores so the server can add types.
    #[serde(other)]
    Unknown,
}

impl fmt::Debug for PrewarmedSandboxControlEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Assignment {
                generation,
                execution_config,
                runtime_env,
                api_key: _,
                expires_at,
            } => f
                .debug_struct("Assignment")
                .field("generation", generation)
                .field("run_id", &execution_config.task_id.inner())
                .field("execution_id", &execution_config.execution_id.inner())
                .field("repository_count", &execution_config.repositories.len())
                .field(
                    "setup_command_count",
                    &execution_config.setup_commands.len(),
                )
                .field(
                    "mcp_config_present",
                    &!execution_config.mcp_servers_json.is_empty(),
                )
                .field("skill_count", &execution_config.skills.len())
                .field("runtime_env_count", &runtime_env.len())
                .field("api_key", &"<redacted>")
                .field("expires_at", expires_at)
                .finish(),
            Self::Retired { reason } => f.debug_struct("Retired").field("reason", reason).finish(),
            Self::Unknown => f.write_str("Unknown"),
        }
    }
}

/// The only message a sandbox sends; the server ignores it once the sandbox is assigned.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PrewarmedSandboxControlRequest<'a> {
    Heartbeat { generation: &'a str },
}

type ControlSink = Pin<Box<dyn futures::Sink<websocket::Message, Error = websocket::Error> + Send>>;
type ControlStream =
    Pin<Box<dyn futures::Stream<Item = Result<websocket::Message, websocket::Error>> + Send>>;

/// Plain JSON control stream for pre-warmed sandbox lease and assignment messages.
pub struct PrewarmedSandboxControlStream {
    sink: ControlSink,
    stream: ControlStream,
}

impl PrewarmedSandboxControlStream {
    pub fn split(self) -> (PrewarmedSandboxControlWriter, PrewarmedSandboxControlReader) {
        (
            PrewarmedSandboxControlWriter { sink: self.sink },
            PrewarmedSandboxControlReader {
                stream: self.stream,
            },
        )
    }
}

/// Write half of the pre-warmed sandbox control stream.
pub struct PrewarmedSandboxControlWriter {
    sink: ControlSink,
}

impl PrewarmedSandboxControlWriter {
    async fn send(&mut self, request: PrewarmedSandboxControlRequest<'_>) -> Result<()> {
        let message = serde_json::to_string(&request)
            .context("Failed to serialize pre-warmed sandbox control message")?;
        self.sink
            .send(websocket::Message::new_text(message))
            .await
            .context("Failed to send pre-warmed sandbox control message")
    }

    pub async fn send_heartbeat(&mut self, generation: &str) -> Result<()> {
        self.send(PrewarmedSandboxControlRequest::Heartbeat { generation })
            .await
    }
}

/// Read half of the pre-warmed sandbox control stream.
pub struct PrewarmedSandboxControlReader {
    stream: ControlStream,
}

impl PrewarmedSandboxControlReader {
    pub async fn next(&mut self) -> Result<Option<PrewarmedSandboxControlEvent>> {
        loop {
            let Some(message) = self.stream.next().await else {
                return Ok(None);
            };
            let message = message.context("Pre-warmed sandbox control stream failed")?;
            // tungstenite hands ping frames to the reader after it queues the pong itself.
            let Some(text) = message.text() else {
                continue;
            };
            return serde_json::from_str(text)
                .context("Failed to decode pre-warmed sandbox control message");
        }
    }
}

/// Endpoints a pre-warmed sandbox calls while it prepares itself and waits to be claimed.
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait PrewarmedSandboxClient: 'static + Send + Sync {
    /// Fetches this sandbox's preparation plan and credentials.
    async fn fetch_configuration(&self, sandbox_id: &str) -> Result<PrewarmedSandboxConfiguration>;

    /// Reports whether preparation succeeded for the supplied configuration generation.
    async fn report_status(
        &self,
        sandbox_id: &str,
        generation: &str,
        state: PrewarmedSandboxState,
    ) -> Result<()>;

    /// Opens the assignment control stream with the current bearer credential, which is the
    /// standby credential until an assignment installs the run's own.
    async fn connect_control_stream(
        &self,
        sandbox_id: &str,
        generation: &str,
    ) -> Result<PrewarmedSandboxControlStream>;
}

impl ServerApi {
    fn prewarmed_sandbox_url(sandbox_id: &str, action: &str) -> String {
        let origin = if action == "ws" {
            ChannelState::prewarmed_sandbox_control_url()
        } else {
            ChannelState::server_root_url()
        };
        format!(
            "{origin}/internal/prewarmed-sandboxes/{}/{action}",
            urlencoding::encode(sandbox_id),
        )
    }

    /// Attaches both credentials the prewarmed endpoints require.
    async fn prewarmed_sandbox_request<'a>(
        &self,
        builder: http_client::RequestBuilder<'a>,
    ) -> Result<http_client::RequestBuilder<'a>> {
        let workload_token = self
            .base_client
            .get_or_create_ambient_workload_token()
            .await?
            .context("A pre-warmed sandbox request requires a workload token")?;
        let mut workload_token = HeaderValue::from_str(&workload_token)
            .context("Invalid pre-warmed sandbox workload token header")?;
        workload_token.set_sensitive(true);
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for a pre-warmed sandbox request")?;
        let token = auth_token
            .as_bearer_token()
            .context("A pre-warmed sandbox request requires an access token")?;
        Ok(builder
            .header(PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER, workload_token)
            .bearer_auth(token))
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl PrewarmedSandboxClient for ServerApi {
    async fn fetch_configuration(&self, sandbox_id: &str) -> Result<PrewarmedSandboxConfiguration> {
        let operation = PrewarmedSandboxConfig::build(PrewarmedSandboxConfigVariables {
            input: PrewarmedSandboxConfigInput {
                sandbox_id: sandbox_id.into(),
            },
            request_context: get_request_context(),
        });
        let response = self.send_graphql_request(operation, None).await?;
        match response.prewarmed_sandbox_config {
            PrewarmedSandboxConfigResult::PrewarmedSandboxConfiguration(config) => {
                (*config).try_into()
            }
            PrewarmedSandboxConfigResult::UserFacingError(_) => {
                bail!("Pre-warmed sandbox configuration is unavailable")
            }
            PrewarmedSandboxConfigResult::Unknown => {
                bail!("Unknown pre-warmed sandbox configuration response")
            }
        }
    }

    async fn report_status(
        &self,
        sandbox_id: &str,
        generation: &str,
        state: PrewarmedSandboxState,
    ) -> Result<()> {
        bounded_request(REQUEST_TIMEOUT, async {
            let url = Self::prewarmed_sandbox_url(sandbox_id, "status");
            let body = PrewarmedSandboxStatusInput {
                fingerprint: generation.to_string(),
                state: state.as_wire_value(),
                manifest_hash: matches!(state, PrewarmedSandboxState::Ready)
                    .then(|| generation.to_string()),
            };
            let response = self
                .prewarmed_sandbox_request(self.base_client.http_client().post(&url).json(&body))
                .await?
                .send()
                .await
                .context("Failed to report the pre-warmed sandbox status")?;
            if !response.status().is_success() {
                return Err(Self::error_from_response(response).await);
            }
            Ok(())
        })
        .await
    }

    async fn connect_control_stream(
        &self,
        sandbox_id: &str,
        generation: &str,
    ) -> Result<PrewarmedSandboxControlStream> {
        let workload_token = self
            .base_client
            .get_or_create_ambient_workload_token()
            .await?
            .context("A pre-warmed sandbox control stream requires a workload token")?;
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for a pre-warmed sandbox control stream")?;
        let token = auth_token
            .as_bearer_token()
            .context("A pre-warmed sandbox control stream requires an access token")?;
        let url = control_socket_url(&Self::prewarmed_sandbox_url(sandbox_id, "ws"), generation)?;
        #[cfg(not(target_family = "wasm"))]
        let socket = websocket::WebSocket::connect(
            control_request(&url, token, &workload_token)?,
            std::iter::empty::<&str>(),
        )
        .await
        .context("Failed to connect the pre-warmed sandbox control stream")?;
        #[cfg(target_family = "wasm")]
        let socket = websocket::WebSocket::connect_with_headers(
            &url,
            std::iter::empty::<&str>(),
            vec![
                (PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER, workload_token),
                (
                    http::header::AUTHORIZATION.as_str(),
                    format!("Bearer {token}"),
                ),
            ],
        )
        .await
        .context("Failed to connect the pre-warmed sandbox control stream")?;
        let (sink, stream) = socket.split().await;
        Ok(PrewarmedSandboxControlStream {
            sink: Box::pin(sink),
            stream: Box::pin(stream),
        })
    }
}

/// Stops the entire request at its deadline so a stalled phase cannot block later requests.
async fn bounded_request<T>(
    timeout: Duration,
    request: impl Future<Output = Result<T>>,
) -> Result<T> {
    request
        .with_timeout(timeout)
        .await
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::TimedOut))
        .context("Pre-warmed sandbox request timed out")?
}

#[cfg(test)]
#[path = "prewarmed_sandbox_tests.rs"]
pub(crate) mod tests;
