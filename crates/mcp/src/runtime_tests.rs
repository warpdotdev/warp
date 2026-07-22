use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rmcp::model::{ErrorCode, ErrorData, Resource, ServerCapabilities, Tool};

use super::{query_resources_for, query_tools_for, should_query_resources, should_query_tools};

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
