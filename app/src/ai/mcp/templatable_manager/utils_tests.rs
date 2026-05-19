#[cfg(test)]
mod tests {
    use crate::ai::mcp::templatable_manager::utils::{
        query_prompts_for, query_resources_for, query_tools_for, should_query_prompts,
        should_query_resources, should_query_tools,
    };
    use rmcp::model::{
        ErrorCode, ErrorData, Prompt, PromptsCapability, Resource, ServerCapabilities, Tool,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Build a `ServerCapabilities` with selected capability flags toggled on.
    /// `Some(default)` mirrors how rmcp deserializes a capability the server
    /// advertised with no inner flags set.
    ///
    /// The builder uses const-generic state so `enable_tools()` can't be
    /// invoked conditionally — each combination needs its own builder chain.
    /// The prompts flag is set on the built value because the builder API
    /// exposes no `enable_prompts()`.
    fn caps(tools: bool, resources: bool, prompts: bool) -> ServerCapabilities {
        let mut caps = match (tools, resources) {
            (true, true) => ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            (true, false) => ServerCapabilities::builder().enable_tools().build(),
            (false, true) => ServerCapabilities::builder().enable_resources().build(),
            (false, false) => ServerCapabilities::builder().build(),
        };
        if prompts {
            caps.prompts = Some(PromptsCapability::default());
        }
        caps
    }

    fn test_prompt(name: &str) -> Prompt {
        Prompt::new(name, Some("test prompt"), None)
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

    // ---------- predicate-level tests ----------

    /// Regression test for warpdotdev/warp#6798: each capability is queried
    /// independently. Previously, asymmetric handling could cause `tools/list`
    /// to be skipped when a server advertised both `tools` and `resources`,
    /// resulting in "No tools available" even though the server had tools.
    #[test]
    fn each_capability_is_queried_independently() {
        for has_tools in [false, true] {
            for has_resources in [false, true] {
                let c = caps(has_tools, has_resources, false);
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

    // ---------- query_tools_for control-flow tests ----------

    /// When `tools` is not advertised, the helper must skip the list call so
    /// we don't waste a round trip and pollute the wire log with a request
    /// that's destined to return `METHOD_NOT_FOUND`.
    #[tokio::test]
    async fn query_tools_for_skips_listing_when_capability_not_advertised() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let no_caps = caps(false, false, false);

        let result = query_tools_for(Some(&no_caps), "srv", || async move {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(vec![test_tool("never")])
        })
        .await;

        assert!(result.is_empty());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "list function must not be called when tools capability is absent",
        );
    }

    /// `None` server info follows the same skip-listing path as "no capability".
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

    /// Happy path: tools advertised, list call succeeds, tools returned.
    #[tokio::test]
    async fn query_tools_for_returns_listed_tools_when_capability_advertised() {
        let c = caps(true, false, false);
        let expected = vec![test_tool("greet"), test_tool("review")];
        let to_return = expected.clone();

        let result = query_tools_for(Some(&c), "srv", || async move { Ok(to_return) }).await;

        assert_eq!(result, expected);
    }

    /// `tools` advertised but server returns an empty list — distinct from the
    /// "skipped" case in that we still made the call.
    #[tokio::test]
    async fn query_tools_for_returns_empty_vec_when_server_lists_no_tools() {
        let c = caps(true, false, false);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();

        let result = query_tools_for(Some(&c), "srv", || async move {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        })
        .await;

        assert!(result.is_empty());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "list function still called when capability is advertised",
        );
    }

    /// **The fail-soft test the bug ticket implicitly demands.** Transport-
    /// closed errors must not abort server startup; the helper must log and
    /// return an empty vec. This is the regression-protector for #6798's
    /// underlying asymmetry — if anyone re-introduces a `return Err(...)` here,
    /// this test fails.
    #[tokio::test]
    async fn query_tools_for_returns_empty_on_transport_error() {
        let c = caps(true, false, false);
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
        let c = caps(true, false, false);
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

    /// The list function must be called exactly once per query — not zero
    /// (that would be the skip path) and not multiple times (no implicit
    /// retry inside the helper).
    #[tokio::test]
    async fn query_tools_for_calls_list_function_exactly_once() {
        let c = caps(true, false, false);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();

        let _ = query_tools_for(Some(&c), "srv", || async move {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(vec![test_tool("p")])
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Independence: tools-listing decision must not depend on whether
    /// resources are also advertised. Run the full happy-path flow under
    /// every (tools, resources) combination and assert tools come back iff
    /// the tools capability is advertised.
    #[tokio::test]
    async fn query_tools_for_decision_independent_of_other_capabilities() {
        let tools = vec![test_tool("x")];
        for has_tools in [false, true] {
            for has_resources in [false, true] {
                let c = caps(has_tools, has_resources, false);
                let to_return = tools.clone();
                let result =
                    query_tools_for(Some(&c), "srv", || async move { Ok(to_return) }).await;

                if has_tools {
                    assert_eq!(
                        result, tools,
                        "expected tools when advertised \
                         (tools={has_tools}, resources={has_resources})",
                    );
                } else {
                    assert!(
                        result.is_empty(),
                        "expected empty when tools not advertised \
                         (tools={has_tools}, resources={has_resources})",
                    );
                }
            }
        }
    }

    // ---------- query_resources_for control-flow tests ----------

    #[tokio::test]
    async fn query_resources_for_skips_listing_when_capability_not_advertised() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let no_caps = caps(false, false, false);

        let result = query_resources_for(Some(&no_caps), "srv", || async move {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(vec![test_resource("file:///nope")])
        })
        .await;

        assert!(result.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

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

    #[tokio::test]
    async fn query_resources_for_returns_listed_resources_when_capability_advertised() {
        let c = caps(false, true, false);
        let expected = vec![test_resource("file:///a"), test_resource("file:///b")];
        let to_return = expected.clone();

        let result = query_resources_for(Some(&c), "srv", || async move { Ok(to_return) }).await;

        assert_eq!(result, expected);
    }

    /// Fail-soft on transport errors — same contract as `query_tools_for`,
    /// matching the existing behavior the predicate refactor preserved.
    #[tokio::test]
    async fn query_resources_for_returns_empty_on_transport_error() {
        let c = caps(false, true, false);
        let result = query_resources_for(Some(&c), "srv", || async {
            Err(rmcp::ServiceError::TransportClosed)
        })
        .await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn query_resources_for_returns_empty_on_mcp_error() {
        let c = caps(false, true, false);
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

    #[tokio::test]
    async fn query_resources_for_calls_list_function_exactly_once() {
        let c = caps(false, true, false);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();

        let _ = query_resources_for(Some(&c), "srv", || async move {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(vec![test_resource("file:///a")])
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    // ---------- should_query_prompts predicate tests ----------

    /// `None` server info (no `peer_info` available) short-circuits to "do not
    /// query": there's no capability declaration to drive the decision.
    #[test]
    fn should_query_prompts_returns_false_when_server_info_is_none() {
        assert!(!should_query_prompts(None));
    }

    /// A server that responded to `initialize` but advertised no capabilities
    /// must not be queried for prompts.
    #[test]
    fn should_query_prompts_returns_false_when_no_capabilities_advertised() {
        let no_caps = caps(false, false, false);
        assert!(!should_query_prompts(Some(&no_caps)));
    }

    /// Decision must depend ONLY on `capabilities.prompts` — never on whether
    /// `tools` or `resources` are also advertised. Exhaustively covers all
    /// 2^3 = 8 combinations.
    #[test]
    fn should_query_prompts_only_depends_on_prompts_field() {
        for has_tools in [false, true] {
            for has_resources in [false, true] {
                for has_prompts in [false, true] {
                    let c = caps(has_tools, has_resources, has_prompts);
                    assert_eq!(
                        should_query_prompts(Some(&c)),
                        has_prompts,
                        "tools={has_tools}, resources={has_resources}, prompts={has_prompts}",
                    );
                }
            }
        }
    }

    /// `PromptsCapability::list_changed` is informational (whether the server
    /// will send `notifications/prompts/list_changed`); it must not affect the
    /// initial-listing decision.
    #[test]
    fn should_query_prompts_ignores_list_changed_value() {
        for list_changed in [None, Some(false), Some(true)] {
            let mut c = ServerCapabilities::builder().build();
            c.prompts = Some(PromptsCapability { list_changed });
            assert!(
                should_query_prompts(Some(&c)),
                "list_changed={list_changed:?} must still query",
            );
        }
    }

    // ---------- query_prompts_for control-flow tests ----------

    /// When the prompts capability is not advertised, the helper must NOT
    /// call the list function (avoids unnecessary RPC and keeps the wire log
    /// clean).
    #[tokio::test]
    async fn query_prompts_for_skips_listing_when_capability_not_advertised() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let no_caps = caps(false, false, false);

        let result = query_prompts_for(Some(&no_caps), "srv", || async move {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(vec![test_prompt("never")])
        })
        .await;

        assert!(result.is_empty(), "no prompts returned when not advertised");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "list function must not be called when capability is absent",
        );
    }

    /// `None` server info follows the same skip-listing path as "no capability".
    #[tokio::test]
    async fn query_prompts_for_skips_listing_when_server_info_is_none() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();

        let result = query_prompts_for(None, "srv", || async move {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(vec![test_prompt("never")])
        })
        .await;

        assert!(result.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// Happy path: capability advertised, list call succeeds, prompts returned.
    #[tokio::test]
    async fn query_prompts_for_returns_listed_prompts_when_capability_advertised() {
        let c = caps(false, false, true);
        let expected = vec![test_prompt("greet"), test_prompt("review")];
        let to_return = expected.clone();

        let result = query_prompts_for(Some(&c), "srv", || async move { Ok(to_return) }).await;

        assert_eq!(result, expected);
    }

    /// Capability advertised but listing is also valid when the server has
    /// zero prompts (empty vec). Distinct from the "skipped" case in that we
    /// still made the call; we just got an empty response.
    #[tokio::test]
    async fn query_prompts_for_returns_empty_vec_when_server_lists_no_prompts() {
        let c = caps(false, false, true);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();

        let result = query_prompts_for(Some(&c), "srv", || async move {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        })
        .await;

        assert!(result.is_empty());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "list function still called when capability is advertised",
        );
    }

    /// Transport-closed errors must fail soft: log + empty vec, no propagation.
    #[tokio::test]
    async fn query_prompts_for_returns_empty_on_transport_error() {
        let c = caps(false, false, true);
        let result = query_prompts_for(Some(&c), "srv", || async {
            Err(rmcp::ServiceError::TransportClosed)
        })
        .await;
        assert!(result.is_empty());
    }

    /// MCP-protocol errors (e.g. METHOD_NOT_FOUND from a server that
    /// advertised the capability but rejects the call) also fail soft.
    #[tokio::test]
    async fn query_prompts_for_returns_empty_on_mcp_error() {
        let c = caps(false, false, true);
        let result = query_prompts_for(Some(&c), "srv", || async {
            Err(rmcp::ServiceError::McpError(ErrorData {
                code: ErrorCode::METHOD_NOT_FOUND,
                message: "prompts/list not implemented".into(),
                data: None,
            }))
        })
        .await;
        assert!(result.is_empty());
    }

    /// The list function must be called exactly once per query — not zero
    /// (we'd skip the work) and not multiple times (no implicit retry).
    #[tokio::test]
    async fn query_prompts_for_calls_list_function_exactly_once() {
        let c = caps(false, false, true);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();

        let _ = query_prompts_for(Some(&c), "srv", || async move {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(vec![test_prompt("p")])
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Independence test: query_prompts_for's behavior must not depend on
    /// whether tools or resources are also advertised. Run the full happy-path
    /// flow under all 8 capability combinations and assert prompts are returned
    /// iff the prompts capability is advertised.
    #[tokio::test]
    async fn query_prompts_for_decision_independent_of_other_capabilities() {
        let prompts = vec![test_prompt("x")];
        for has_tools in [false, true] {
            for has_resources in [false, true] {
                for has_prompts in [false, true] {
                    let c = caps(has_tools, has_resources, has_prompts);
                    let to_return = prompts.clone();
                    let result =
                        query_prompts_for(Some(&c), "srv", || async move { Ok(to_return) }).await;

                    if has_prompts {
                        assert_eq!(
                            result, prompts,
                            "expected prompts to be returned when advertised \
                             (tools={has_tools}, resources={has_resources}, prompts={has_prompts})",
                        );
                    } else {
                        assert!(
                            result.is_empty(),
                            "expected empty when prompts not advertised \
                             (tools={has_tools}, resources={has_resources})",
                        );
                    }
                }
            }
        }
    }

    /// Sanity: a `Prompt` constructed via the rmcp API survives passage
    /// through `query_prompts_for` unchanged. Catches accidental mapping
    /// or dropping if the helper ever grows transformation logic.
    #[tokio::test]
    async fn query_prompts_for_preserves_prompt_fields_unchanged() {
        let c = caps(false, false, true);
        let original = Prompt::new(
            "code-review",
            Some("Review the diff at HEAD against the spec"),
            None,
        );
        let to_return = vec![original.clone()];

        let result = query_prompts_for(Some(&c), "srv", || async move { Ok(to_return) }).await;

        assert_eq!(result.len(), 1);
        assert_eq!(result[0], original);
    }
}
