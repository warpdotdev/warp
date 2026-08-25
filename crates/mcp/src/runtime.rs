//! Capability-gating helpers used during MCP server startup.
//!
//! Each `query_*_for` function pairs a capability check with the actual list
//! call from rmcp, gating the call on advertisement and failing soft on errors.
//! They take the list call as a closure so unit tests can drive the gate-and-
//! fail-soft control flow with a fake `RunningService` substitute.

use std::collections::HashMap;
use std::future::Future;

use cfg_if::cfg_if;
use cloud_object_models::{StaticEnvVar, TransportType};
use futures::FutureExt as _;
use rmcp::ServiceExt as _;
use rmcp::transport::ConfigureCommandExt as _;
use simple_logger::SimpleLogger;
use tokio::io::AsyncBufReadExt as _;
use uuid::Uuid;
use warp_errors::report_error;

use super::TemplatableMCPServerInfo;
use crate::error_classification::{ProxyAuthReason, parse_www_authenticate_reason};

type ReqwestHttpTransport = rmcp::transport::StreamableHttpClientTransport<reqwest::Client>;
type ReqwestSseTransport = crate::sse_transport::SseClientTransport<reqwest::Client>;

/// Error from [`spawn_server`], distinguishing authentication failures from
/// other spawn failures so callers can react without string matching.
#[derive(Debug, thiserror::Error)]
#[allow(clippy::large_enum_variant)] // `Other` carries rmcp's (large) error type.
pub enum McpSpawnError {
    /// The server rejected the connection as unauthenticated, and interactive
    /// authentication was unavailable or failed.
    #[error("{message}")]
    AuthRequired {
        /// The `WWW-Authenticate` challenge, when the server provided one.
        www_authenticate: Option<String>,
        /// The Warp proxy re-mint reason parsed from the challenge, if any.
        /// `Some` means a freshly minted proxy session token will fix this
        /// without user interaction.
        reason: Option<ProxyAuthReason>,
        /// User-facing description of the failure.
        message: String,
    },
    #[error(transparent)]
    Other(#[from] rmcp::RmcpError),
}

impl From<rmcp::service::ClientInitializeError> for McpSpawnError {
    fn from(error: rmcp::service::ClientInitializeError) -> Self {
        Self::Other(error.into())
    }
}

/// Whether a spawn failure justifies deleting cached OAuth credentials.
///
/// Only a definitive authentication rejection that re-minting cannot fix
/// qualifies. Transient failures (network, DNS, command-not-found) and
/// re-mintable proxy-token expiry must never log the user out of a server.
pub fn should_delete_credentials(error: &McpSpawnError) -> bool {
    matches!(error, McpSpawnError::AuthRequired { reason: None, .. })
}

/// Convert a spawn error to a user-friendly error message.
pub fn spawn_error_to_user_message(error: &McpSpawnError) -> String {
    match error {
        McpSpawnError::AuthRequired { message, .. } => message.clone(),
        McpSpawnError::Other(error) => error_to_user_message(error),
    }
}

/// Convert an rmcp error to a user-friendly error message.
pub fn error_to_user_message(error: &rmcp::RmcpError) -> String {
    match error {
        rmcp::RmcpError::ClientInitialize(err) => {
            format!("Failed to initialize client: {}", err)
        }
        rmcp::RmcpError::ServerInitialize(err) => {
            format!("Failed to initialize server: {}", err)
        }
        rmcp::RmcpError::TransportCreation { error, .. } => {
            format!("Failed to establish connection: {}", error)
        }
        rmcp::RmcpError::Runtime(err) => {
            format!("Runtime error: {}", err)
        }
        rmcp::RmcpError::Service(err) => match err {
            rmcp::ServiceError::McpError(_) => {
                "Server returned an error. Please check server logs for details.".to_string()
            }
            rmcp::ServiceError::TransportSend(_) => {
                "Failed to send data to server. Connection may have been lost.".to_string()
            }
            rmcp::ServiceError::TransportClosed => {
                "Connection closed unexpectedly. The server may have crashed.".to_string()
            }
            rmcp::ServiceError::UnexpectedResponse => {
                "Server sent an unexpected response. The server may be incompatible.".to_string()
            }
            rmcp::ServiceError::Cancelled { reason } => format!(
                "Operation was cancelled with reason: {}",
                reason.clone().unwrap_or("Unknown reason".to_string())
            ),
            rmcp::ServiceError::Timeout { timeout } => {
                format!(
                    "Connection timed out after {} seconds. The server may be unresponsive.",
                    timeout.as_secs()
                )
            }
            _ => format!("Service error: {}", err),
        },
        // The enum is marked as non-exhaustive, so we need a catch-all.
        _ => {
            format!("Error: {error}")
        }
    }
}

/// Bounded reconnect policy for legacy SSE streams.
///
/// The upstream default retries forever every second, hammering a dead or
/// auth-rejecting endpoint indefinitely. Exhaustion ends the stream, which
/// surfaces as a closed transport that higher layers can recover from
/// deliberately.
fn sse_retry_policy() -> std::sync::Arc<dyn crate::sse_transport::SseRetryPolicy> {
    std::sync::Arc::new(crate::sse_transport::ExponentialBackoff {
        max_times: Some(6),
        base_duration: std::time::Duration::from_millis(1000),
    })
}

/// Builds a `HeaderMap` from a `HashMap<String, String>` of user-provided headers.
///
/// Invalid header names or values are skipped.
fn build_header_map(headers: &HashMap<String, String>) -> reqwest::header::HeaderMap {
    headers.try_into().unwrap_or_default()
}

/// Builds a reqwest client with custom headers for MCP HTTP/SSE connections.
#[allow(clippy::result_large_err)]
pub fn build_client_with_headers(
    headers: &HashMap<String, String>,
) -> Result<reqwest::Client, rmcp::RmcpError> {
    let header_map = build_header_map(headers);

    reqwest::Client::builder()
        .default_headers(header_map)
        .build()
        .map_err(|e| {
            rmcp::RmcpError::transport_creation::<ReqwestHttpTransport>(format!(
                "Failed to build client with headers: {e}",
            ))
        })
}

/// Spawns a new MCP server from a given [`TransportType`].
#[allow(clippy::result_large_err)]
pub async fn spawn_server(
    server_name: String,
    description: Option<String>,
    uuid: Uuid,
    transport_type: TransportType,
    logger: SimpleLogger,
    auth_context: Option<crate::oauth::AuthContext>,
) -> Result<TemplatableMCPServerInfo, McpSpawnError> {
    logger.log("[note] Attention! There may be sensitive information (such as API keys) in these logs. Make sure to redact any secrets before sharing with others.".to_string());

    // Every transport is wrapped in `TransportLoggingWrapper`, which flips
    // this to true when the connection dies; see
    // `TemplatableMCPServerInfo::transport_closed`.
    let (closed_tx, transport_closed) = tokio::sync::watch::channel(false);
    let mut is_authenticated_transport = false;
    let service = match transport_type {
        TransportType::CLIServer(cli_server) => {
            logger.log("[info] MCP: Using stdio transport".to_string());

            cfg_if! {
                if #[cfg(windows)] {
                    // We wrap the command in cmd.exe /c to allow Windows to be responsible for resolving the
                    // PATH variable rather than depending on the `Command` implementation, which only looks for
                    // `.exe` files in directories found in PATH.
                    // https://github.com/rust-lang/rust/issues/37519
                    let command = "cmd.exe".to_owned();
                    let args = std::iter::once("/c".to_owned())
                        .chain(std::iter::once(cli_server.command))
                        .chain(cli_server.args)
                        .collect::<Vec<String>>();
                } else {
                    let command = cli_server.command;
                    let args = cli_server.args;
                }
            }

            // Capture the command and configured cwd for diagnostics before they're
            // moved into the Command builder closure.
            let command_for_log = command.clone();
            let cwd_for_log = cli_server.cwd_parameter.clone();

            // Try to spawn the child process.
            let (transport, stderr) = rmcp::transport::TokioChildProcess::builder(
                tokio::process::Command::new(command).configure(|cmd| {
                    cmd.args(args);
                    if let Some(cwd) = cli_server.cwd_parameter {
                        cmd.current_dir(cwd);
                    }
                    for StaticEnvVar { name, value } in cli_server.static_env_vars.iter() {
                        if value.is_empty() {
                            // Skip empty/unset environment variables so that, in the CLI, they can be inherited.
                            logger.log(format!(
                                "[warn] MCP: Skipping empty environment variable: {name}"
                            ));
                            continue;
                        }
                        cmd.env(name, value);
                    }

                    // On Windows, ensure that no console window is shown.
                    #[cfg(windows)]
                    cmd.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
                }),
            )
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    let cwd_display = cwd_for_log
                        .as_deref()
                        .unwrap_or("<inherited from Warp's process cwd>");
                    logger.log(format!(
                        "[error] MCP: Failed to spawn '{server_name}': command '{command_for_log}' \
                         not found (cwd: {cwd_display}). If your MCP server depends on a specific \
                         working directory, set the `working_directory` field in your config to \
                         override the default."
                    ));
                }
                rmcp::RmcpError::transport_creation::<rmcp::transport::TokioChildProcess>(err)
            })?;

            let pid = transport
                .id()
                .map(|pid| pid.to_string())
                .unwrap_or("??".to_string());

            // We always expect to have an stderr, but this is marginally safer than unwrapping.
            if let Some(stderr) = stderr {
                let logger = logger.clone();
                // Spawn a background task to forward from the child process's stderr to our logger.
                tokio::spawn(async move {
                    let mut buf = String::new();
                    let mut reader = tokio::io::BufReader::new(stderr);
                    loop {
                        match reader.read_line(&mut buf).await {
                            // EOF.
                            Ok(0) => return,
                            // Read some data.
                            Ok(_) => logger.log(format!("[info] MCP [pid: {pid}] stderr: {buf}")),
                            // Failed to read from the child process's stderr.
                            Err(e) => {
                                report_error!(
                                    anyhow::Error::new(e).context("Failed to read stderr")
                                );
                                return;
                            }
                        }
                    }
                });
            }

            // Wrap the transport in a logging wrapper.
            let transport = TransportLoggingWrapper {
                transport,
                logger: logger.clone(),
                closed_tx: closed_tx.clone(),
            };

            // Create the MCP client and connect to the server.
            Ok::<_, McpSpawnError>(make_client_info().into_dyn().serve(transport).await?)
        }
        TransportType::ServerSentEvents(sse_server) => {
            let headers: HashMap<String, String> = sse_server
                .headers
                .iter()
                .map(|h| (h.name.clone(), h.value.clone()))
                .collect();
            match determine_transport(server_name.clone(), &sse_server.url, &headers, auth_context)
                .await
            {
                // TODO: these need headers also?
                Ok(Transport::Http(Some(client))) => {
                    is_authenticated_transport = true;

                    logger.log("[info] MCP: Using Streaming HTTP transport".to_string());
                    let transport = rmcp::transport::StreamableHttpClientTransport::with_client(
                        client,
                        rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                            sse_server.url.clone(),
                        ),
                    );
                    let transport = TransportLoggingWrapper {
                        transport,
                        logger: logger.clone(),
                        closed_tx: closed_tx.clone(),
                    };
                    Ok(make_client_info().into_dyn().serve(transport).await?)
                }
                Ok(Transport::Http(None)) => {
                    logger.log("[info] MCP: Using Streaming HTTP transport".to_string());
                    let transport = if headers.is_empty() {
                        rmcp::transport::StreamableHttpClientTransport::from_uri(
                            sse_server.url.clone(),
                        )
                    } else {
                        let client = build_client_with_headers(&headers)?;
                        rmcp::transport::StreamableHttpClientTransport::with_client(
                            client,
                            rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                                sse_server.url.clone(),
                            ),
                        )
                    };
                    let transport = TransportLoggingWrapper {
                        transport,
                        logger: logger.clone(),
                        closed_tx: closed_tx.clone(),
                    };
                    Ok(make_client_info().into_dyn().serve(transport).await?)
                }
                Ok(Transport::Sse(Some(client))) => {
                    is_authenticated_transport = true;

                    logger.log("[info] MCP: Using (legacy) SSE transport (due to preflight failing with a 404)".to_string());
                    let transport = crate::sse_transport::SseClientTransport::start_with_client(
                        client,
                        crate::sse_transport::SseClientConfig {
                            sse_endpoint: sse_server.url.into(),
                            retry_policy: sse_retry_policy(),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(rmcp::RmcpError::transport_creation::<ReqwestSseTransport>)?;
                    let transport = TransportLoggingWrapper {
                        transport,
                        logger: logger.clone(),
                        closed_tx: closed_tx.clone(),
                    };
                    Ok(make_client_info().into_dyn().serve(transport).await?)
                }
                Ok(Transport::Sse(None)) => {
                    logger.log("[info] MCP: Using (legacy) SSE transport (due to preflight failing with a 404)".to_string());
                    let client = if headers.is_empty() {
                        reqwest::Client::default()
                    } else {
                        build_client_with_headers(&headers)?
                    };
                    let transport = crate::sse_transport::SseClientTransport::start_with_client(
                        client,
                        crate::sse_transport::SseClientConfig {
                            sse_endpoint: sse_server.url.clone().into(),
                            retry_policy: sse_retry_policy(),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(rmcp::RmcpError::transport_creation::<ReqwestSseTransport>)?;
                    let transport = TransportLoggingWrapper {
                        transport,
                        logger: logger.clone(),
                        closed_tx: closed_tx.clone(),
                    };
                    Ok(make_client_info().into_dyn().serve(transport).await?)
                }
                Err(err) => {
                    logger.log(format!(
                        "[error] MCP: preflight connection to MCP server failed: {err:#}"
                    ));
                    Err(err)?
                }
            }
        }
    }?;

    let server_info = service.peer_info();
    logger.log(format!("[info] MCP: Connected to server: {server_info:#?}"));

    let capabilities = server_info.map(|info| &info.capabilities);

    let resources =
        query_resources_for(capabilities, &server_name, || service.list_all_resources()).await;
    let tools = query_tools_for(capabilities, &server_name, || service.list_all_tools()).await;

    Ok(TemplatableMCPServerInfo {
        name: server_name,
        service,
        resources,
        tools,
        installation_id: uuid,
        description,
        is_authenticated_transport,
        transport_closed,
    })
}

/// The transport to use for MCP.
enum Transport {
    /// The HTTP transport, with an optional authenticated client.
    Http(Option<rmcp::transport::auth::AuthClient<reqwest::Client>>),
    /// The SSE transport, with an optional authenticated client.
    Sse(Option<rmcp::transport::auth::AuthClient<reqwest::Client>>),
}

/// Determines which transport to use.
///
/// This sends a "preflight" InitializeRequest to the server to determine whether the
/// server supports the HTTP transport (or needs to use the SSE transport), and if
/// authentication is required.
#[allow(clippy::result_large_err)]
async fn determine_transport(
    server_name: String,
    url: &str,
    headers: &HashMap<String, String>,
    auth_context: Option<crate::oauth::AuthContext>,
) -> Result<Transport, McpSpawnError> {
    use reqwest::StatusCode;

    fn unexpected_error(status: reqwest::StatusCode) -> McpSpawnError {
        rmcp::RmcpError::transport_creation::<ReqwestHttpTransport>(format!(
            "Unexpected status code: {status}"
        ))
        .into()
    }
    let probe = send_initialize_request(url, headers, None).await?;
    match probe.status {
        StatusCode::OK => Ok(Transport::Http(None)),
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => Ok(Transport::Sse(None)),
        StatusCode::UNAUTHORIZED => {
            // An expired/stale Warp proxy session is not an OAuth problem:
            // routing it into the interactive OAuth flow would show a
            // misleading "authenticate this server" prompt. Surface it as a
            // re-mintable auth failure instead.
            if let Some(reason) = probe
                .www_authenticate
                .as_deref()
                .and_then(parse_www_authenticate_reason)
            {
                return Err(McpSpawnError::AuthRequired {
                    www_authenticate: probe.www_authenticate,
                    reason: Some(reason),
                    message: "The Warp proxy session for this MCP server has expired.".to_string(),
                });
            }
            let Some(mut auth_context) = auth_context else {
                return Err(McpSpawnError::AuthRequired {
                    www_authenticate: probe.www_authenticate,
                    reason: None,
                    message: "Server requires authentication, which is not yet supported."
                        .to_string(),
                });
            };

            // Grab the post-authentication callback so we can invoke it once we know for sure that we successfully
            // went through the OAuth flow for a server and were able to successfully send an initialize request.
            let authenticated_callback = std::mem::take(&mut auth_context.authenticated);

            // Go through the OAuth flow to get an authenticated client.
            // This will first attempt to use cached credentials before starting interactive OAuth.
            let http_client = build_client_with_headers(headers)?;
            let (client, did_require_login) =
                crate::oauth::make_authenticated_client(url, http_client, auth_context)
                    .await
                    .map_err(|error| McpSpawnError::AuthRequired {
                        www_authenticate: None,
                        reason: None,
                        message: format!("{error:#}"),
                    })?;

            // Define a helper function to invoke when we've successfully authenticated.
            let emit_authenticated_notification = async move || {
                if did_require_login
                    && let Some(authenticated_callback) = authenticated_callback
                    && let Err(err) = authenticated_callback(server_name).await
                {
                    log::warn!("Failed to emit MCP authenticated notification: {err:?}");
                }
            };

            match send_initialize_request(url, headers, Some(&client))
                .await?
                .status
            {
                StatusCode::OK => {
                    emit_authenticated_notification().await;
                    Ok(Transport::Http(Some(client)))
                }
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => {
                    emit_authenticated_notification().await;
                    Ok(Transport::Sse(Some(client)))
                }
                other => Err(unexpected_error(other)),
            }
        }
        status => Err(unexpected_error(status)),
    }
}

/// The observable outcome of a preflight InitializeRequest.
struct InitializeProbe {
    status: reqwest::StatusCode,
    /// The `WWW-Authenticate` challenge on a 401, used to distinguish a
    /// re-mintable Warp proxy-session expiry from a real OAuth requirement.
    www_authenticate: Option<String>,
}

/// Sends an InitializeRequest to the server, and returns the HTTP status code
/// (plus auth challenge, if any) from the response.
#[allow(clippy::result_large_err)]
async fn send_initialize_request(
    url: &str,
    headers: &HashMap<String, String>,
    auth_client: Option<&rmcp::transport::auth::AuthClient<reqwest::Client>>,
) -> Result<InitializeProbe, rmcp::RmcpError> {
    use rmcp::transport::common::http_header::{EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE};

    let request = rmcp::model::InitializeRequest::new(make_client_info());
    let request = rmcp::model::ClientJsonRpcMessage::request(
        rmcp::model::ClientRequest::InitializeRequest(request),
        rmcp::model::RequestId::Number(0),
    );

    let mut request = build_client_with_headers(headers)?
        .post(url)
        .header(
            http::header::ACCEPT,
            [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "),
        )
        .json(&request);

    if let Some(auth_client) = auth_client.as_ref() {
        let access_token = auth_client
            .get_access_token()
            .await
            .map_err(rmcp::RmcpError::transport_creation::<ReqwestHttpTransport>)?;
        request = request.bearer_auth(access_token);
    }

    let response = request
        .send()
        .await
        .map_err(rmcp::RmcpError::transport_creation::<ReqwestHttpTransport>)?;

    Ok(InitializeProbe {
        status: response.status(),
        www_authenticate: response
            .headers()
            .get(http::header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
    })
}

/// Creates a [`ClientInfo`] for the MCP client.
///
/// This tells the MCP server who we are and what capabilities we have.
fn make_client_info() -> rmcp::model::ClientInfo {
    rmcp::model::ClientInfo::new(
        Default::default(),
        rmcp::model::Implementation::new(
            warp_core::channel::ChannelState::app_id().to_string(),
            warp_core::channel::ChannelState::app_version()
                .map(|v| v.to_string())
                .unwrap_or_default(),
        ),
    )
}

/// Whether to query `resources/list` for a server with the given capabilities.
///
/// Per the MCP spec, the client should only invoke a list method when the server
/// has advertised the corresponding capability during initialization.
fn should_query_resources(capabilities: Option<&rmcp::model::ServerCapabilities>) -> bool {
    capabilities.is_some_and(|c| c.resources.is_some())
}

/// Whether to query `tools/list` for a server with the given capabilities.
///
/// Per the MCP spec, the client should only invoke a list method when the server
/// has advertised the corresponding capability during initialization.
fn should_query_tools(capabilities: Option<&rmcp::model::ServerCapabilities>) -> bool {
    capabilities.is_some_and(|c| c.tools.is_some())
}

/// Query `resources/list` for a connected MCP server.
///
/// Skips the call entirely when `resources` was not advertised. Treats any
/// listing error as "no resources" (fail-soft) so a flaky `resources/list`
/// does not abort the entire server startup. Mirrors the behavior of
/// [`query_tools_for`] so the two capabilities are handled symmetrically.
async fn query_resources_for<F, Fut>(
    capabilities: Option<&rmcp::model::ServerCapabilities>,
    server_name: &str,
    list_resources: F,
) -> Vec<rmcp::model::Resource>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<rmcp::model::Resource>, rmcp::ServiceError>>,
{
    if !should_query_resources(capabilities) {
        return Vec::new();
    }
    match list_resources().await {
        Ok(result) => result,
        Err(err) => {
            log::warn!("Failed to list resources for MCP server '{server_name}': {err}");
            Vec::new()
        }
    }
}

/// Query `tools/list` for a connected MCP server.
///
/// Skips the call entirely when `tools` was not advertised. Treats any listing
/// error as "no tools" (fail-soft) so a transient `tools/list` failure does
/// not abort the entire server startup — the user-visible regression #6798
/// was rooted in the prior asymmetric handling, where a tools-list error on
/// a server with healthy resources would propagate and fail startup.
async fn query_tools_for<F, Fut>(
    capabilities: Option<&rmcp::model::ServerCapabilities>,
    server_name: &str,
    list_tools: F,
) -> Vec<rmcp::model::Tool>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<rmcp::model::Tool>, rmcp::ServiceError>>,
{
    if !should_query_tools(capabilities) {
        return Vec::new();
    }
    match list_tools().await {
        Ok(result) => result,
        Err(err) => {
            log::warn!("Failed to list tools for MCP server '{server_name}': {err}");
            Vec::new()
        }
    }
}

/// A wrapper around a [`rmcp::transport::Transport`] that logs all requests
/// and responses, and signals transport death through a watch channel.
struct TransportLoggingWrapper<T> {
    transport: T,
    logger: SimpleLogger,
    /// Flipped to `true` when `receive` observes end-of-input or the
    /// transport is closed.
    closed_tx: tokio::sync::watch::Sender<bool>,
}

impl<T: rmcp::transport::Transport<R>, R: rmcp::service::ServiceRole> rmcp::transport::Transport<R>
    for TransportLoggingWrapper<T>
{
    type Error = T::Error;

    fn send(
        &mut self,
        item: rmcp::service::TxJsonRpcMessage<R>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        if let Ok(json) = serde_json::to_string(&item) {
            self.logger
                .log(format!("[info] MCP: Sending request: {json}"));
        }

        let logger = self.logger.clone();
        self.transport.send(item).map(move |result| {
            if let Err(e) = &result {
                logger.log(format!("[warn] MCP: Failed to send request: {e:#}"));
            }
            result
        })
    }

    fn receive(
        &mut self,
    ) -> impl Future<Output = Option<rmcp::service::RxJsonRpcMessage<R>>> + Send {
        let logger = self.logger.clone();
        async move {
            let result = self.transport.receive().await;
            match &result {
                Some(item) => {
                    if let Ok(json) = serde_json::to_string(item) {
                        logger.log(format!("[info] MCP: Received response: {json}"));
                    }
                }
                None => {
                    // End of input: the server hung up or the child died.
                    let _ = self.closed_tx.send(true);
                }
            }
            result
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = self.closed_tx.send(true);
        self.transport.close()
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
