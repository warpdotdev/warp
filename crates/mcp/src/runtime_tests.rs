use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::post;
use rmcp::model::{ErrorCode, ErrorData, Resource, ServerCapabilities, Tool};

use super::{
    MAX_STDERR_LINE_BYTES, challenge_parameter_names, determine_transport,
    forward_stderr_to_logger, has_caller_supplied_credential, is_oauth_challenge,
    query_resources_for, query_tools_for, should_query_resources, should_query_tools,
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

/// Wraps an in-memory byte slice as an `AsyncBufRead` fake stderr stream.
fn fake_stderr(data: impl Into<Vec<u8>>) -> tokio::io::BufReader<std::io::Cursor<Vec<u8>>> {
    tokio::io::BufReader::new(std::io::Cursor::new(data.into()))
}

/// Each logged entry must contain only its own line, not the cumulative
/// stderr history (APP-5349).
#[tokio::test]
async fn forward_stderr_to_logger_does_not_accumulate_across_lines() {
    let reader = fake_stderr(*b"first\nsecond\nthird\n");
    let logged: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let logged_clone = logged.clone();

    forward_stderr_to_logger(reader, "123", move |msg| {
        logged_clone.lock().unwrap().push(msg);
    })
    .await;

    assert_eq!(
        *logged.lock().unwrap(),
        vec![
            "[info] MCP [pid: 123] stderr: first".to_string(),
            "[info] MCP [pid: 123] stderr: second".to_string(),
            "[info] MCP [pid: 123] stderr: third".to_string(),
        ]
    );
}

/// A trailing, non-newline-terminated chunk is still flushed at EOF.
#[tokio::test]
async fn forward_stderr_to_logger_flushes_trailing_content_without_newline_at_eof() {
    let reader = fake_stderr(*b"no newline at end");
    let logged: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let logged_clone = logged.clone();

    forward_stderr_to_logger(reader, "42", move |msg| {
        logged_clone.lock().unwrap().push(msg);
    })
    .await;

    assert_eq!(
        *logged.lock().unwrap(),
        vec!["[info] MCP [pid: 42] stderr: no newline at end".to_string()]
    );
}

/// Every logged entry stays roughly the size of a single line, regardless of
/// how many lines have already been forwarded.
#[tokio::test]
async fn forward_stderr_to_logger_many_lines_does_not_grow_each_log_call() {
    let num_lines = 500;
    let mut input = Vec::new();
    for i in 0..num_lines {
        input.extend_from_slice(format!("line-{i}\n").as_bytes());
    }
    let reader = fake_stderr(input);
    let logged: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let logged_clone = logged.clone();

    forward_stderr_to_logger(reader, "99", move |msg| {
        logged_clone.lock().unwrap().push(msg);
    })
    .await;

    let logged = logged.lock().unwrap();
    assert_eq!(logged.len(), num_lines);

    let max_len = logged.iter().map(|m| m.len()).max().unwrap();
    let min_len = logged.iter().map(|m| m.len()).min().unwrap();
    assert!(
        max_len - min_len < 10,
        "logged entries grew across iterations instead of staying roughly line-sized: min={min_len} max={max_len}"
    );
    assert_eq!(
        logged.last().unwrap(),
        &format!("[info] MCP [pid: 99] stderr: line-{}", num_lines - 1)
    );
}

/// A line that never terminates in a newline is still bounded: once it
/// accumulates `MAX_STDERR_LINE_BYTES`, it's force-flushed and reset.
#[tokio::test]
async fn forward_stderr_to_logger_bounds_a_single_unterminated_line() {
    let total_len = MAX_STDERR_LINE_BYTES * 3;
    let input = vec![b'a'; total_len];
    let reader = fake_stderr(input.clone());
    let logged: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let logged_clone = logged.clone();

    forward_stderr_to_logger(reader, "7", move |msg| {
        logged_clone.lock().unwrap().push(msg);
    })
    .await;

    let logged = logged.lock().unwrap();
    let prefix = "[info] MCP [pid: 7] stderr: ";

    assert!(
        logged.len() > 1,
        "expected the oversized, newline-free line to be split into multiple bounded \
         flushes, got {} log call(s)",
        logged.len()
    );
    for msg in logged.iter() {
        let content = msg
            .strip_prefix(prefix)
            .expect("message should have the expected prefix");
        assert!(
            content.len() <= MAX_STDERR_LINE_BYTES,
            "a single flushed chunk exceeded the cap: {} bytes",
            content.len()
        );
    }

    let reconstructed: String = logged
        .iter()
        .map(|msg| msg.strip_prefix(prefix).unwrap())
        .collect();
    assert_eq!(reconstructed, String::from_utf8(input).unwrap());
}

/// Force-flushing at `MAX_STDERR_LINE_BYTES` must not split a multi-byte
/// UTF-8 character across two chunks; the incomplete trailing bytes are
/// carried over and combined with the rest before decoding.
#[tokio::test]
async fn forward_stderr_to_logger_does_not_split_utf8_scalar_at_cap() {
    let text = format!("{}\u{20ac}", "a".repeat(MAX_STDERR_LINE_BYTES - 1));
    let mut input = text.clone().into_bytes();
    input.push(b'\n');

    let reader = fake_stderr(input);
    let logged: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let logged_clone = logged.clone();

    forward_stderr_to_logger(reader, "1", move |msg| {
        logged_clone.lock().unwrap().push(msg);
    })
    .await;

    let logged = logged.lock().unwrap();
    let prefix = "[info] MCP [pid: 1] stderr: ";

    for msg in logged.iter() {
        assert!(
            !msg.contains('\u{fffd}'),
            "logged chunk contains a UTF-8 replacement character: {msg:?}"
        );
    }

    let reconstructed: String = logged
        .iter()
        .map(|msg| msg.strip_prefix(prefix).unwrap())
        .collect();
    assert_eq!(reconstructed, text);
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
