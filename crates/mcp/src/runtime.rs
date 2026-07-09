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
use rmcp::transport::ConfigureCommandExt as _;
use rmcp::ServiceExt as _;
use simple_logger::SimpleLogger;
use tokio::io::AsyncBufReadExt as _;
use uuid::Uuid;
use warp_core::report_error;

use super::TemplatableMCPServerInfo;

type ReqwestHttpTransport = rmcp::transport::StreamableHttpClientTransport<reqwest::Client>;
type ReqwestSseTransport = crate::sse_transport::SseClientTransport<reqwest::Client>;

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
) -> Result<TemplatableMCPServerInfo, rmcp::RmcpError> {
    logger.log("[note] Attention! There may be sensitive information (such as API keys) in these logs. Make sure to redact any secrets before sharing with others.".to_string());

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
            };

            // Create the MCP client and connect to the server.
            Ok::<_, rmcp::RmcpError>(make_client_info().into_dyn().serve(transport).await?)
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
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(rmcp::RmcpError::transport_creation::<ReqwestSseTransport>)?;
                    let transport = TransportLoggingWrapper {
                        transport,
                        logger: logger.clone(),
                    };
                    Ok(make_client_info().into_dyn().serve(transport).await?)
                }
                Ok(Transport::Sse(None)) => {
                    logger.log("[info] MCP: Using (legacy) SSE transport (due to preflight failing with a 404)".to_string());
                    let transport = if headers.is_empty() {
                        crate::sse_transport::SseClientTransport::start(sse_server.url.clone())
                            .await
                            .map_err(|e| {
                                rmcp::RmcpError::transport_creation::<ReqwestSseTransport>(e)
                            })?
                    } else {
                        let client = build_client_with_headers(&headers)?;
                        crate::sse_transport::SseClientTransport::start_with_client(
                            client,
                            crate::sse_transport::SseClientConfig {
                                sse_endpoint: sse_server.url.clone().into(),
                                ..Default::default()
                            },
                        )
                        .await
                        .map_err(rmcp::RmcpError::transport_creation::<ReqwestSseTransport>)?
                    };
                    let transport = TransportLoggingWrapper {
                        transport,
                        logger: logger.clone(),
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
) -> Result<Transport, rmcp::RmcpError> {
    use reqwest::StatusCode;

    fn unexpected_error(status: reqwest::StatusCode) -> rmcp::RmcpError {
        rmcp::RmcpError::transport_creation::<ReqwestHttpTransport>(format!(
            "Unexpected status code: {status}"
        ))
    }
    match send_initialize_request(url, headers, None).await? {
        StatusCode::OK => Ok(Transport::Http(None)),
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => Ok(Transport::Sse(None)),
        StatusCode::UNAUTHORIZED => {
            let Some(mut auth_context) = auth_context else {
                return Err(rmcp::RmcpError::transport_creation::<ReqwestHttpTransport>(
                    "Server requires authentication, which is not yet supported.".to_string(),
                ));
            };

            // Grab the post-authentication callback so we can invoke it once we know for sure that we successfully
            // went through the OAuth flow for a server and were able to successfully send an initialize request.
            let authenticated_callback = std::mem::take(&mut auth_context.authenticated);

            // Go through the OAuth flow to get an authenticated client.
            // This will first attempt to use cached credentials before starting interactive OAuth.
            let (client, did_require_login) =
                crate::oauth::make_authenticated_client(url, auth_context)
                    .await
                    .map_err(rmcp::RmcpError::transport_creation::<ReqwestHttpTransport>)?;

            // Define a helper function to invoke when we've successfully authenticated.
            let emit_authenticated_notification = async move || {
                if did_require_login {
                    if let Some(authenticated_callback) = authenticated_callback {
                        if let Err(err) = authenticated_callback(server_name).await {
                            log::warn!("Failed to emit MCP authenticated notification: {err:?}");
                        }
                    }
                }
            };

            match send_initialize_request(url, headers, Some(&client)).await? {
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

/// Sends an InitializeRequest to the server, and returns the HTTP status code from the response.
#[allow(clippy::result_large_err)]
async fn send_initialize_request(
    url: &str,
    headers: &HashMap<String, String>,
    auth_client: Option<&rmcp::transport::auth::AuthClient<reqwest::Client>>,
) -> Result<reqwest::StatusCode, rmcp::RmcpError> {
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

    Ok(response.status())
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

/// A wrapper around a [`rmcp::transport::Transport`] that logs all requests and responses.
struct TransportLoggingWrapper<T> {
    transport: T,
    logger: SimpleLogger,
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
            if let Some(item) = &result {
                if let Ok(json) = serde_json::to_string(item) {
                    logger.log(format!("[info] MCP: Received response: {json}"));
                }
            }
            result
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.transport.close()
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
