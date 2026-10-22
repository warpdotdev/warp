use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use cloud_object_models::{ServerSentEvents, StaticHeader, TransportType};
use rmcp::model::{
    CallToolRequestParams, DiscoverResult, ErrorCode, ErrorData, Resource, ServerCapabilities, Tool,
};
use serde_json::{Value, json};

use super::{
    challenge_parameter_names, determine_transport, has_caller_supplied_credential,
    is_oauth_challenge, query_resources_for, query_tools_for, should_query_resources,
    should_query_tools, spawn_server,
};

/// Build a `ServerCapabilities` with selected capability flags toggled on.
/// Each `Some(default)` mirrors how rmcp deserializes a capability the
/// server advertised with no inner flags set.
fn caps(tools: bool, resources: bool) -> ServerCapabilities {
    match (tools, resources) {
        (true, true) => ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build(),
        (true, false) => ServerCapabilities::builder().enable_tools().build(),
        (false, true) => ServerCapabilities::builder().enable_resources().build(),
        (false, false) => ServerCapabilities::builder().build(),
    }
}
fn legacy_version_error(version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "server-error",
        "error": {
            "code": ErrorCode::INVALID_REQUEST,
            "message": format!(
                "Bad Request: Unsupported protocol version: {version}. Supported versions: \
                 2024-11-05, 2025-03-26, 2025-06-18, 2025-11-25"
            ),
        },
    })
}

fn test_tool(name: &str) -> Tool {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "description": "test tool",
        "inputSchema": { "type": "object" },
    }))
    .expect("Tool deserialization")
}

fn test_resource(uri: &str) -> Resource {
    serde_json::from_value(serde_json::json!({
        "uri": uri,
        "name": "test resource",
    }))
    .expect("Resource deserialization")
}

/// Regression test for warpdotdev/warp#6798: each capability is queried
/// independently. Previously, asymmetric handling could cause `tools/list`
/// to be skipped when a server advertised both `tools` and `resources`,
/// resulting in "No tools available" even though the server had tools.
#[test]
fn each_capability_is_queried_independently() {
    for has_tools in [false, true] {
        for has_resources in [false, true] {
            let c = caps(has_tools, has_resources);
            assert_eq!(
                should_query_tools(Some(&c)),
                has_tools,
                "tools={has_tools}, resources={has_resources}",
            );
            assert_eq!(
                should_query_resources(Some(&c)),
                has_resources,
                "tools={has_tools}, resources={has_resources}",
            );
        }
    }
    assert!(!should_query_tools(None));
    assert!(!should_query_resources(None));
}

/// When `tools` is not advertised, the helper must skip the list call so
/// we don't waste a round trip and pollute the wire log with a request
/// that's destined to return `METHOD_NOT_FOUND`.
#[tokio::test]
async fn query_tools_for_skips_listing_when_capability_not_advertised() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = calls.clone();
    let no_caps = caps(false, false);

    let result = query_tools_for(Some(&no_caps), "srv", || async move {
        calls_clone.fetch_add(1, Ordering::SeqCst);
        Ok(vec![test_tool("never")])
    })
    .await;

    assert!(result.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// Skips `tools/list` when server info is absent.
#[tokio::test]
async fn query_tools_for_skips_listing_when_server_info_is_none() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = calls.clone();

    let result = query_tools_for(None, "srv", || async move {
        calls_clone.fetch_add(1, Ordering::SeqCst);
        Ok(vec![test_tool("never")])
    })
    .await;

    assert!(result.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// Returns listed tools when `tools` is advertised.
#[tokio::test]
async fn query_tools_for_returns_listed_tools_when_capability_advertised() {
    let c = caps(true, false);
    let expected = vec![test_tool("greet"), test_tool("review")];
    let to_return = expected.clone();

    let result = query_tools_for(Some(&c), "srv", || async move { Ok(to_return) }).await;

    assert_eq!(result, expected);
}

/// Returns an empty vector when the server lists no tools.
#[tokio::test]
async fn query_tools_for_returns_empty_vec_when_server_lists_no_tools() {
    let c = caps(true, false);
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = calls.clone();

    let result = query_tools_for(Some(&c), "srv", || async move {
        calls_clone.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    })
    .await;

    assert!(result.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// **The fail-soft test the bug ticket implicitly demands.** Transport-
/// closed errors must not abort server startup; the helper must log and
/// return an empty vec. This is the regression-protector for #6798's
/// underlying asymmetry — if anyone re-introduces a `return Err(...)` here,
/// this test fails.
#[tokio::test]
async fn query_tools_for_returns_empty_on_transport_error() {
    let c = caps(true, false);
    let result = query_tools_for(Some(&c), "srv", || async {
        Err(rmcp::ServiceError::TransportClosed)
    })
    .await;
    assert!(result.is_empty());
}

/// MCP-protocol errors (e.g. METHOD_NOT_FOUND from a misbehaving server
/// that advertised the capability but rejects the call) also fail soft,
/// so the rest of the server surface still comes up.
#[tokio::test]
async fn query_tools_for_returns_empty_on_mcp_error() {
    let c = caps(true, false);
    let result = query_tools_for(Some(&c), "srv", || async {
        Err(rmcp::ServiceError::McpError(ErrorData {
            code: ErrorCode::METHOD_NOT_FOUND,
            message: "tools/list not implemented".into(),
            data: None,
        }))
    })
    .await;
    assert!(result.is_empty());
}

/// Calls the `tools/list` function exactly once per query.
#[tokio::test]
async fn query_tools_for_calls_list_function_exactly_once() {
    let c = caps(true, false);
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = calls.clone();

    let _ = query_tools_for(Some(&c), "srv", || async move {
        calls_clone.fetch_add(1, Ordering::SeqCst);
        Ok(vec![test_tool("p")])
    })
    .await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// Keeps the tools-listing decision independent of resource capability state.
#[tokio::test]
async fn query_tools_for_decision_independent_of_other_capabilities() {
    let tools = vec![test_tool("x")];
    for has_tools in [false, true] {
        for has_resources in [false, true] {
            let c = caps(has_tools, has_resources);
            let to_return = tools.clone();
            let result = query_tools_for(Some(&c), "srv", || async move { Ok(to_return) }).await;

            if has_tools {
                assert_eq!(result, tools);
            } else {
                assert!(result.is_empty());
            }
        }
    }
}

/// Skips `resources/list` when `resources` is not advertised.
#[tokio::test]
async fn query_resources_for_skips_listing_when_capability_not_advertised() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = calls.clone();
    let no_caps = caps(false, false);

    let result = query_resources_for(Some(&no_caps), "srv", || async move {
        calls_clone.fetch_add(1, Ordering::SeqCst);
        Ok(vec![test_resource("file:///nope")])
    })
    .await;

    assert!(result.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// Skips `resources/list` when server info is absent.
#[tokio::test]
async fn query_resources_for_skips_listing_when_server_info_is_none() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = calls.clone();

    let result = query_resources_for(None, "srv", || async move {
        calls_clone.fetch_add(1, Ordering::SeqCst);
        Ok(vec![test_resource("file:///nope")])
    })
    .await;

    assert!(result.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// Returns listed resources when `resources` is advertised.
#[tokio::test]
async fn query_resources_for_returns_listed_resources_when_capability_advertised() {
    let c = caps(false, true);
    let expected = vec![test_resource("file:///a"), test_resource("file:///b")];
    let to_return = expected.clone();

    let result = query_resources_for(Some(&c), "srv", || async move { Ok(to_return) }).await;

    assert_eq!(result, expected);
}

/// Fails soft when `resources/list` sees a transport error.
#[tokio::test]
async fn query_resources_for_returns_empty_on_transport_error() {
    let c = caps(false, true);
    let result = query_resources_for(Some(&c), "srv", || async {
        Err(rmcp::ServiceError::TransportClosed)
    })
    .await;
    assert!(result.is_empty());
}

/// Fails soft when `resources/list` returns an MCP protocol error.
#[tokio::test]
async fn query_resources_for_returns_empty_on_mcp_error() {
    let c = caps(false, true);
    let result = query_resources_for(Some(&c), "srv", || async {
        Err(rmcp::ServiceError::McpError(ErrorData {
            code: ErrorCode::METHOD_NOT_FOUND,
            message: "resources/list not implemented".into(),
            data: None,
        }))
    })
    .await;
    assert!(result.is_empty());
}

/// Calls the `resources/list` function exactly once per query.
#[tokio::test]
async fn query_resources_for_calls_list_function_exactly_once() {
    let c = caps(false, true);
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = calls.clone();

    let _ = query_resources_for(Some(&c), "srv", || async move {
        calls_clone.fetch_add(1, Ordering::SeqCst);
        Ok(vec![test_resource("file:///a")])
    })
    .await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

// --- 401 disambiguation ---

fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Only a protected-resource challenge means "begin OAuth". A plain rejection
/// of a bearer token must not be read as one, since that is what sends a user
/// looking for an OAuth flow that does not exist.
#[test]
fn oauth_challenge_requires_resource_metadata() {
    let challenge =
        r#"Bearer resource_metadata="https://srv.example/.well-known/oauth-protected-resource""#;
    assert!(is_oauth_challenge(challenge));
    // Header names and parameters are case-insensitive on the wire.
    assert!(is_oauth_challenge(
        r#"bearer RESOURCE_METADATA="https://srv.example""#
    ));
    assert!(is_oauth_challenge(
        r#"Bearer resource_metadata = "https://srv.example""#
    ));

    assert!(!is_oauth_challenge("Bearer"));
    assert!(!is_oauth_challenge(
        r#"Bearer error="invalid_token", error_description="expired""#
    ));
    assert!(!is_oauth_challenge("Basic realm=\"srv\""));
}

/// A quoted parameter value that merely mentions `resource_metadata` is still
/// a rejection. Matching the raw string rather than the parameter name would
/// route this back into OAuth and undo the whole disambiguation.
#[test]
fn oauth_challenge_ignores_resource_metadata_inside_quoted_values() {
    assert!(!is_oauth_challenge(
        r#"Bearer error="invalid_token", error_description="no resource_metadata was supplied""#
    ));
    assert!(!is_oauth_challenge(r#"Bearer realm="resource_metadata""#));
    assert!(!is_oauth_challenge(
        r#"Bearer error_description="mentions \" resource_metadata=\"fake\"", error="invalid_token""#
    ));
    // Still detected when a genuine parameter follows a decoy quoted value.
    assert!(is_oauth_challenge(
        r#"Bearer error_description="mentions resource_metadata", resource_metadata="https://srv.example""#
    ));
}

/// Parameter-name extraction has to skip quoted values and tolerate malformed
/// input without inventing names.
#[test]
fn challenge_parameter_names_skips_quoted_values() {
    assert_eq!(
        challenge_parameter_names(r#"Bearer realm="srv", error="invalid_token""#),
        vec!["realm", "error"]
    );
    assert_eq!(
        challenge_parameter_names("Bearer scope=mcp, error=invalid_token"),
        vec!["scope", "error"]
    );
    assert!(challenge_parameter_names("Bearer").is_empty());
    // An unterminated quote must not loop or fabricate further parameters.
    assert_eq!(
        challenge_parameter_names(r#"Bearer realm="unterminated"#),
        vec!["realm"]
    );
}

/// A caller-supplied credential is what makes a bare 401 mean "your token was
/// rejected" rather than "authenticate with me".
#[test]
fn caller_supplied_credential_detects_configured_auth_headers() {
    assert!(has_caller_supplied_credential(&headers(&[(
        "Authorization",
        "Bearer tok"
    )])));
    // Header names arrive in whatever case the user typed them.
    assert!(has_caller_supplied_credential(&headers(&[(
        "authorization",
        "Bearer tok"
    )])));
    assert!(has_caller_supplied_credential(&headers(&[(
        "X-Api-Key",
        "abc123"
    )])));

    assert!(!has_caller_supplied_credential(&headers(&[])));
    assert!(!has_caller_supplied_credential(&headers(&[(
        "Content-Type",
        "application/json"
    )])));
    // An empty credential is not a credential; treating it as one would
    // suppress the OAuth path for a server that legitimately needs it.
    assert!(!has_caller_supplied_credential(&headers(&[(
        "Authorization",
        "   "
    )])));
}

async fn serve_http_router(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("fake MCP server should bind");
    let url = format!(
        "http://{}/mcp",
        listener
            .local_addr()
            .expect("fake MCP address should resolve")
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("fake MCP server should run");
    });
    (url, server)
}

#[derive(Clone, Copy)]
enum FixtureProtocol {
    Discover,
    InitializeCounteroffer,
    InitializeVersionError,
    LegacyVersionError,
}

fn rpc_error(request: &Value, code: ErrorCode, data: Option<Value>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request["id"],
        "error": { "code": code, "message": "Request rejected", "data": data },
    })
}

async fn start_http_server(
    url: String,
    headers: Vec<StaticHeader>,
) -> Result<super::TemplatableMCPServerInfo, rmcp::RmcpError> {
    let mut logs = simple_logger::manager::LogManager::new();
    logs.register_namespace("mcp-tests", false);
    let logger = logs
        .register(
            "mcp-tests",
            std::env::temp_dir().join(format!("mcp-{}.log", uuid::Uuid::new_v4())),
            Arc::new(warpui::r#async::executor::Background::default()),
        )
        .expect("test logger");
    tokio::time::timeout(
        Duration::from_secs(5),
        spawn_server(
            "test-server".to_string(),
            None,
            uuid::Uuid::new_v4(),
            None,
            TransportType::ServerSentEvents(ServerSentEvents { url, headers }),
            logger,
            None,
        ),
    )
    .await
    .expect("Warp startup should complete")
}

async fn assert_http_startup(
    protocol: FixtureProtocol,
    version: &'static str,
    sse_responses: bool,
    static_credential: bool,
) -> Vec<Value> {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let app = Router::new().route("/mcp", post(move |headers: HeaderMap, Json(request): Json<Value>| {
        captured.lock().expect("request capture").push(request.clone());
        async move {
            if static_credential
                && (headers.get("x-api-key").and_then(|v| v.to_str().ok()) != Some("test-key")
                    || headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer test-token")) {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            let requested_version = headers.get("mcp-protocol-version")
                .and_then(|v| v.to_str().ok())
                .or_else(|| request["params"]["protocolVersion"].as_str());
            if matches!(protocol, FixtureProtocol::LegacyVersionError)
                && requested_version.is_some_and(|requested| requested != version)
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(legacy_version_error(requested_version.expect("rejected version"))),
                ).into_response();
            }
            let method = request["method"].as_str().expect("request method");
            let result = match method {
                "server/discover" => {
                    let requested = &request["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"];
                    if matches!(protocol, FixtureProtocol::InitializeCounteroffer) {
                        rpc_error(&request, ErrorCode::METHOD_NOT_FOUND, None)
                    } else if requested != version {
                        rpc_error(&request, ErrorCode::UNSUPPORTED_PROTOCOL_VERSION,
                            Some(json!({ "requested": requested, "supported": [version] })))
                    } else if matches!(protocol, FixtureProtocol::Discover) {
                        let result = DiscoverResult::new(
                            vec![serde_json::from_value(json!(version)).expect("fixture version")],
                            caps(true, false),
                        );
                        json!({ "jsonrpc": "2.0", "id": request["id"], "result": result })
                    } else {
                        rpc_error(&request, ErrorCode::METHOD_NOT_FOUND, None)
                    }
                }
                "initialize" => {
                    if matches!(protocol, FixtureProtocol::Discover) {
                        return StatusCode::BAD_REQUEST.into_response();
                    }
                    if matches!(protocol, FixtureProtocol::InitializeVersionError)
                        && request["params"]["protocolVersion"] != version {
                        rpc_error(&request, ErrorCode::INVALID_PARAMS, Some(json!({ "supported": [version] })))
                    } else {
                        json!({
                            "jsonrpc": "2.0", "id": request["id"],
                            "result": {
                                "protocolVersion": version, "capabilities": { "tools": {} },
                                "serverInfo": { "name": "legacy", "version": "1.0" },
                            },
                        })
                    }
                }
                "notifications/initialized" => return StatusCode::ACCEPTED.into_response(),
                "tools/list" => {
                    if headers.get("mcp-protocol-version").and_then(|v| v.to_str().ok()) != Some(version) {
                        return StatusCode::BAD_REQUEST.into_response();
                    }
                    if matches!(protocol, FixtureProtocol::Discover)
                        && (request["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] != version
                            || !request["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"].is_object()
                            || (version == "2026-07-28" && headers.get("mcp-method").and_then(|v| v.to_str().ok()) != Some("tools/list")))
                    {
                        return StatusCode::BAD_REQUEST.into_response();
                    }
                    json!({ "jsonrpc": "2.0", "id": request["id"], "result": { "tools": [test_tool("query_metrics")] } })
                }
                "tools/call" => {
                    assert_eq!(headers.get("mcp-protocol-version").and_then(|v| v.to_str().ok()), Some(version));
                    assert_eq!(request["params"]["name"], "query_metrics");
                    json!({ "jsonrpc": "2.0", "id": request["id"], "result": {
                        "content": [{ "type": "text", "text": "metrics available" }], "isError": false,
                    } })
                }
                _ => return StatusCode::BAD_REQUEST.into_response(),
            };
            if sse_responses {
                (
                    [(http::header::CONTENT_TYPE, "text/event-stream")],
                    format!(": keepalive\n\nevent: message\ndata: {result}\n\n"),
                ).into_response()
            } else {
                Json(result).into_response()
            }
        }
    }));
    let (url, server) = serve_http_router(app).await;
    let headers = if static_credential {
        vec![
            StaticHeader {
                name: "X-Api-Key".to_string(),
                value: "test-key".to_string(),
            },
            StaticHeader {
                name: "Authorization".to_string(),
                value: "Bearer test-token".to_string(),
            },
        ]
    } else {
        Vec::new()
    };
    let info = start_http_server(url, headers).await;
    let info = info.expect("Warp startup should succeed");
    assert_eq!(info.tools, vec![test_tool("query_metrics")]);
    assert_eq!(
        info.service
            .peer_info()
            .expect("server info")
            .protocol_version
            .as_str(),
        version
    );
    if matches!(protocol, FixtureProtocol::LegacyVersionError) {
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            info.service
                .call_tool(CallToolRequestParams::new("query_metrics")),
        )
        .await
        .expect("tool call should complete")
        .expect("tool call should succeed");
        assert_eq!(result.is_error, Some(false));
        assert_eq!(
            serde_json::to_value(result.content).expect("tool content"),
            json!([{ "type": "text", "text": "metrics available" }])
        );
    }
    info.service.cancel().await.expect("cancel service");
    server.abort();
    requests.lock().expect("captured requests").clone()
}

#[tokio::test]
async fn http_startup_recovers_from_legacy_transport_version_rejection_and_calls_tool() {
    for sse_responses in [false, true] {
        let requests = assert_http_startup(
            FixtureProtocol::LegacyVersionError,
            "2025-11-25",
            sse_responses,
            true,
        )
        .await;
        assert_eq!(
            requests
                .iter()
                .map(|r| r["method"].as_str().expect("method"))
                .collect::<Vec<_>>(),
            vec![
                "server/discover",
                "initialize",
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/call"
            ],
        );
        assert_eq!(requests[1]["params"]["protocolVersion"], "2025-11-25");
        assert_eq!(requests[2]["params"]["protocolVersion"], "2025-11-25");
    }
}

#[tokio::test]
async fn preflight_does_not_relax_correlation_for_other_transport_errors() {
    for (status, id, code, message) in [
        (
            StatusCode::BAD_REQUEST,
            json!(99),
            ErrorCode::INVALID_REQUEST,
            "Bad Request: Unsupported protocol version: 2026-07-28. Supported versions: 2025-11-25",
        ),
        (
            StatusCode::BAD_REQUEST,
            json!("server-error"),
            ErrorCode::INVALID_REQUEST,
            "Bad Request: Missing session ID",
        ),
        (
            StatusCode::BAD_REQUEST,
            json!("server-error"),
            ErrorCode::INVALID_REQUEST,
            "Bad Request: Unsupported protocol version: 2099-01-01. Supported versions: 2025-11-25",
        ),
        (
            StatusCode::BAD_REQUEST,
            json!("server-error"),
            ErrorCode::INTERNAL_ERROR,
            "Bad Request: Unsupported protocol version: 2026-07-28. Supported versions: 2025-11-25",
        ),
        (
            StatusCode::OK,
            json!("server-error"),
            ErrorCode::INVALID_REQUEST,
            "Bad Request: Unsupported protocol version: 2026-07-28. Supported versions: 2025-11-25",
        ),
    ] {
        let requests = Arc::new(AtomicUsize::new(0));
        let captured = requests.clone();
        let app = Router::new().route(
            "/mcp",
            post(move || {
                captured.fetch_add(1, Ordering::SeqCst);
                let id = id.clone();
                async move {
                    (
                        status,
                        Json(json!({
                            "jsonrpc": "2.0", "id": id,
                            "error": { "code": code, "message": message },
                        })),
                    )
                }
            }),
        );
        let (url, server) = serve_http_router(app).await;
        let result = determine_transport("test".to_string(), &url, &HashMap::new(), None).await;
        server.abort();
        assert!(matches!(
            result,
            Err(rmcp::RmcpError::ClientInitialize(
                rmcp::service::ClientInitializeError::UncorrelatedErrorResponse { .. }
            ))
        ));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn preflight_does_not_repeat_legacy_transport_version_fallback() {
    for status in [
        StatusCode::BAD_REQUEST,
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        let requests = Arc::new(AtomicUsize::new(0));
        let captured = requests.clone();
        let app = Router::new().route(
            "/mcp",
            post(move |headers: HeaderMap| {
                captured.fetch_add(1, Ordering::SeqCst);
                async move {
                    (
                        status,
                        Json(legacy_version_error(
                            headers
                                .get("mcp-protocol-version")
                                .and_then(|v| v.to_str().ok())
                                .expect("protocol header"),
                        )),
                    )
                }
            }),
        );
        let (url, server) = serve_http_router(app).await;
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            determine_transport("test".to_string(), &url, &HashMap::new(), None),
        )
        .await
        .expect("negotiation should terminate");
        server.abort();
        assert!(result.is_err());
        assert_eq!(
            requests.load(Ordering::SeqCst),
            if status == StatusCode::BAD_REQUEST {
                2
            } else {
                1
            }
        );
    }
}

#[tokio::test]
async fn http_startup_uses_modern_discovery_and_request_metadata() {
    let requests = assert_http_startup(FixtureProtocol::Discover, "2026-07-28", false, false).await;
    assert!(
        !requests
            .iter()
            .any(|r| r["method"] == "initialize" || r["method"] == "notifications/initialized")
    );
}

#[tokio::test]
async fn http_startup_retries_discovery_at_server_supported_revision_without_initializing() {
    let requests = assert_http_startup(FixtureProtocol::Discover, "2025-11-25", false, false).await;
    assert!(!requests.iter().any(|r| r["method"] == "initialize"));
    assert_eq!(
        requests[0]["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
        "2026-07-28"
    );
    assert_eq!(
        requests[1]["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
        "2025-11-25"
    );
}

#[tokio::test]
async fn http_startup_falls_back_and_propagates_sse_version_error_selection_with_static_headers() {
    let requests = assert_http_startup(
        FixtureProtocol::InitializeVersionError,
        "2025-06-18",
        true,
        true,
    )
    .await;
    let initialize = requests
        .iter()
        .filter(|r| r["method"] == "initialize")
        .collect::<Vec<_>>();
    assert_eq!(initialize.len(), 3);
    assert_eq!(initialize[0]["params"]["protocolVersion"], "2025-11-25");
    assert_eq!(initialize[1]["params"]["protocolVersion"], "2025-06-18");
    assert_eq!(initialize[2]["params"]["protocolVersion"], "2025-06-18");
}

#[tokio::test]
async fn http_startup_passes_legacy_counteroffer_to_real_sdk_connection() {
    let requests = assert_http_startup(
        FixtureProtocol::InitializeCounteroffer,
        "2025-03-26",
        false,
        false,
    )
    .await;
    let initialize = requests
        .iter()
        .filter(|r| r["method"] == "initialize")
        .collect::<Vec<_>>();
    assert_eq!(
        initialize
            .iter()
            .map(|r| r["params"]["protocolVersion"]
                .as_str()
                .expect("protocol version"))
            .collect::<Vec<_>>(),
        vec!["2025-11-25", "2025-03-26"],
    );
}

#[tokio::test]
async fn preflight_preserves_modern_rejections_and_http_auth_or_server_errors() {
    for (status, code) in [
        (
            StatusCode::FORBIDDEN,
            ErrorCode::UNSUPPORTED_PROTOCOL_VERSION,
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::UNSUPPORTED_PROTOCOL_VERSION,
        ),
        (StatusCode::BAD_REQUEST, ErrorCode::HEADER_MISMATCH),
        (
            StatusCode::BAD_REQUEST,
            ErrorCode::MISSING_REQUIRED_CLIENT_CAPABILITY,
        ),
    ] {
        let requests = Arc::new(AtomicUsize::new(0));
        let captured = requests.clone();
        let app = Router::new().route(
            "/mcp",
            post(move |Json(request): Json<Value>| {
                captured.fetch_add(1, Ordering::SeqCst);
                async move { (status, Json(rpc_error(&request, code, None))) }
            }),
        );
        let (url, server) = serve_http_router(app).await;
        let result = determine_transport("test".to_string(), &url, &HashMap::new(), None).await;
        server.abort();
        assert!(result.is_err(), "status {status}, code {code:?}");
        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "must not try initialize or another version"
        );
    }
}

#[tokio::test]
async fn preflight_rejects_unknown_or_exhausted_supported_versions_without_initializing() {
    for (supported, expected_requests) in [
        (json!(["2099-01-01"]), 1),
        (json!(["2025-11-25", "2026-07-28"]), 2),
    ] {
        let requests = Arc::new(AtomicUsize::new(0));
        let captured = requests.clone();
        let app = Router::new().route(
            "/mcp",
            post(move |Json(request): Json<Value>| {
                captured.fetch_add(1, Ordering::SeqCst);
                let supported = supported.clone();
                async move {
                    assert_eq!(request["method"], "server/discover");
                    Json(rpc_error(
                        &request,
                        ErrorCode::UNSUPPORTED_PROTOCOL_VERSION,
                        Some(json!({ "supported": supported })),
                    ))
                }
            }),
        );
        let (url, server) = serve_http_router(app).await;
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            determine_transport("test".to_string(), &url, &HashMap::new(), None),
        )
        .await
        .expect("negotiation should terminate");
        server.abort();
        assert!(matches!(
            result,
            Err(rmcp::RmcpError::ClientInitialize(
                rmcp::service::ClientInitializeError::NoCompatibleProtocolVersion { .. }
            ))
        ));
        assert_eq!(requests.load(Ordering::SeqCst), expected_requests);
    }
}

#[tokio::test]
async fn preflight_rejects_unknown_legacy_counteroffer_before_real_startup() {
    let requests = Arc::new(AtomicUsize::new(0));
    let captured = requests.clone();
    let app = Router::new().route(
        "/mcp",
        post(move |Json(request): Json<Value>| {
            captured.fetch_add(1, Ordering::SeqCst);
            async move {
                let reply = if request["method"] == "server/discover" {
                    rpc_error(&request, ErrorCode::METHOD_NOT_FOUND, None)
                } else {
                    assert_eq!(request["method"], "initialize");
                    json!({ "jsonrpc": "2.0", "id": request["id"], "result": {
                    "protocolVersion": "2099-01-01", "capabilities": {},
                    "serverInfo": { "name": "unknown-version", "version": "1" },
                } })
                };
                Json(reply)
            }
        }),
    );
    let (url, server) = serve_http_router(app).await;
    let result = start_http_server(url, Vec::new()).await;
    server.abort();
    assert!(matches!(
        result,
        Err(rmcp::RmcpError::ClientInitialize(
            rmcp::service::ClientInitializeError::NoCompatibleProtocolVersion { .. }
        ))
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn preflight_rejects_uncorrelated_version_errors_without_retrying() {
    let requests = Arc::new(AtomicUsize::new(0));
    let captured = requests.clone();
    let app = Router::new().route(
        "/mcp",
        post(move || {
            captured.fetch_add(1, Ordering::SeqCst);
            async {
                Json(rpc_error(
                    &json!({ "id": 99 }),
                    ErrorCode::UNSUPPORTED_PROTOCOL_VERSION,
                    Some(json!({ "supported": ["2025-11-25"] })),
                ))
            }
        }),
    );
    let (url, server) = serve_http_router(app).await;
    let result = determine_transport("test".to_string(), &url, &HashMap::new(), None).await;
    server.abort();
    assert!(matches!(
        result,
        Err(rmcp::RmcpError::ClientInitialize(
            rmcp::service::ClientInitializeError::UncorrelatedErrorResponse { .. }
        ))
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn preflight_routes_legacy_endpoint_not_found_to_deprecated_sse() {
    let methods = Arc::new(Mutex::new(Vec::new()));
    let captured = methods.clone();
    let app = Router::new().route(
        "/mcp",
        post(move |Json(request): Json<Value>| {
            captured
                .lock()
                .expect("request capture")
                .push(request["method"].clone());
            async move {
                (
                    StatusCode::NOT_FOUND,
                    Json(rpc_error(&request, ErrorCode::METHOD_NOT_FOUND, None)),
                )
            }
        }),
    );
    let (url, server) = serve_http_router(app).await;
    let result = determine_transport("test".to_string(), &url, &HashMap::new(), None).await;
    server.abort();
    assert!(matches!(result, Ok(super::Transport::Sse(None))));
    assert_eq!(
        *methods.lock().expect("captured requests"),
        vec![json!("server/discover"), json!("initialize")]
    );
}

#[tokio::test]
async fn preflight_bounds_sse_body_before_complete_event_arrives() {
    let app = Router::new().route(
        "/mcp",
        post(|| async {
            (
                [(http::header::CONTENT_TYPE, "text/event-stream")],
                format!("data: {}", "x".repeat(1024 * 1024)),
            )
        }),
    );
    let (url, server) = serve_http_router(app).await;
    let result = determine_transport("test".to_string(), &url, &HashMap::new(), None).await;
    server.abort();
    let message = transport_error_message(result, "oversized response must be rejected");
    assert!(message.contains("exceeds 1 MiB"), "{message}");
}

/// Serves a single `401` with the requested `WWW-Authenticate` values and
/// counts requests so a test can assert none were sent.
async fn serve_401(
    www_authenticate: &'static [&'static str],
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("fake MCP server should bind");
    let url = format!(
        "http://{}/mcp",
        listener
            .local_addr()
            .expect("fake MCP address should resolve")
    );

    let requests = Arc::new(AtomicUsize::new(0));
    let requests_for_handler = requests.clone();
    let app = Router::new().route(
        "/mcp",
        post(move || {
            let requests = requests_for_handler.clone();
            async move {
                requests.fetch_add(1, Ordering::SeqCst);
                let mut response = axum::http::StatusCode::UNAUTHORIZED.into_response();
                for challenge in www_authenticate {
                    response.headers_mut().append(
                        axum::http::header::WWW_AUTHENTICATE,
                        axum::http::HeaderValue::from_static(challenge),
                    );
                }
                response
            }
        }),
    );
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (url, requests, server)
}

/// `Transport` owns a live HTTP client and is deliberately not `Debug`, so
/// `expect_err` is unavailable; this unwraps the error side explicitly.
fn transport_error_message(
    result: Result<super::Transport, rmcp::RmcpError>,
    expectation: &str,
) -> String {
    match result {
        Ok(_) => panic!("{expectation}"),
        Err(error) => format!("{error}"),
    }
}

/// A static credential was configured and the server rejected it with a bare
/// `401`, so the transport should surface the credential rejection and never
/// enter the OAuth flow.
#[tokio::test]
async fn bare_401_with_configured_credential_reports_rejection_not_oauth() {
    let (url, requests, server) = serve_401(&[]).await;

    let message = transport_error_message(
        determine_transport(
            "internal-db-mcp".to_string(),
            &url,
            &headers(&[("Authorization", "Bearer rejected-token")]),
            // `None` stands in for "OAuth is not usable here". If the branch
            // wrongly fell through, the error would be the generic
            // authentication-unsupported message instead.
            None,
        )
        .await,
        "a rejected credential should not produce a transport",
    );
    assert!(
        message.contains("rejected the configured credentials"),
        "expected a credential-rejection error, got: {message}"
    );
    assert!(
        !message.contains("not yet supported"),
        "should not have fallen through to the OAuth path: {message}"
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    server.abort();
}

/// Every authentication challenge is considered before classifying a `401`.
#[tokio::test]
async fn oauth_challenge_in_repeated_header_routes_to_oauth() {
    let (url, _requests, server) = serve_401(&[
        r#"Bearer error="invalid_token""#,
        r#"Bearer resource_metadata = "https://srv.example/.well-known/oauth-protected-resource""#,
    ])
    .await;

    let message = transport_error_message(
        determine_transport(
            "oauth-server".to_string(),
            &url,
            &headers(&[("Authorization", "Bearer stale-token")]),
            None,
        )
        .await,
        "no auth context means the OAuth path cannot complete",
    );
    assert!(
        message.contains("not yet supported"),
        "expected the OAuth path to be taken, got: {message}"
    );

    server.abort();
}

/// A genuine protected-resource challenge still routes into OAuth, even when a
/// stale credential header happens to be configured.
#[tokio::test]
async fn oauth_challenge_still_routes_to_oauth_despite_configured_credential() {
    let (url, _requests, server) = serve_401(&[
        r#"Bearer resource_metadata="https://srv.example/.well-known/oauth-protected-resource""#,
    ])
    .await;

    let message = transport_error_message(
        determine_transport(
            "oauth-server".to_string(),
            &url,
            &headers(&[("Authorization", "Bearer stale-token")]),
            None,
        )
        .await,
        "no auth context means the OAuth path cannot complete",
    );
    assert!(
        message.contains("not yet supported"),
        "expected the OAuth path to be taken, got: {message}"
    );

    server.abort();
}

/// With no caller credential to blame, a bare `401` keeps its previous
/// meaning so servers that under-specify their challenge still work.
#[tokio::test]
async fn bare_401_without_configured_credential_still_attempts_oauth() {
    let (url, _requests, server) = serve_401(&[]).await;

    let message = transport_error_message(
        determine_transport("srv".to_string(), &url, &headers(&[]), None).await,
        "no auth context means the OAuth path cannot complete",
    );
    assert!(
        message.contains("not yet supported"),
        "expected the OAuth path to be taken, got: {message}"
    );

    server.abort();
}

/// Runtime credential values are opaque because their contents may legitimately
/// contain text that resembles a template reference.
#[tokio::test]
async fn credential_header_contents_are_not_parsed_as_template_references() {
    let (url, requests, server) = serve_401(&[]).await;

    let message = transport_error_message(
        determine_transport(
            "internal-db-mcp".to_string(),
            &url,
            &headers(&[("Authorization", "Bearer literal-{{token_fragment}}")]),
            None,
        )
        .await,
        "a rejected credential should not produce a transport",
    );
    assert!(
        message.contains("rejected the configured credentials"),
        "expected a credential-rejection error, got: {message}"
    );
    assert!(
        !message.contains("token_fragment"),
        "credential contents must not appear in the error: {message}"
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    server.abort();
}
