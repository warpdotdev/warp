use super::*;
use crate::queries::get_conversation_usage::{
    ApplyFileDiffStats, CategoryTokenBreakdown, TokenCostBreakdown, ToolCallStats,
};

fn tool_call_stats(count: i32) -> ToolCallStats {
    ToolCallStats { count }
}

fn sample_tool_usage_metadata() -> ToolUsageMetadata {
    ToolUsageMetadata {
        run_command_stats: tool_call_stats(4),
        run_commands_executed: 3,
        read_files_stats: tool_call_stats(7),
        search_codebase_stats: tool_call_stats(0),
        grep_stats: tool_call_stats(2),
        file_glob_stats: tool_call_stats(0),
        call_mcp_tool_stats: tool_call_stats(0),
        read_mcp_resource_stats: tool_call_stats(0),
        suggest_plan_stats: tool_call_stats(0),
        suggest_create_plan_stats: tool_call_stats(0),
        write_to_long_running_shell_command_stats: tool_call_stats(0),
        apply_file_diff_stats: ApplyFileDiffStats {
            count: 5,
            lines_added: 120,
            lines_removed: 40,
            files_changed: 6,
        },
        read_shell_command_output_stats: tool_call_stats(0),
        use_computer_stats: tool_call_stats(0),
    }
}

/// Restored conversations must carry per-model token usage (with per-category
/// breakdowns) and tool usage stats through the GraphQL conversion, so the
/// credits-expansion "Models" rows and tool-call stats render the same as for
/// live conversations.
#[test]
fn conversion_populates_token_usage_and_tool_usage_metadata() {
    let gql = ConversationUsageMetadata {
        context_window_usage: 0.42,
        context_window_segments: vec![ContextWindowSegment {
            segment_type: ContextWindowSegmentType::SystemPrompt,
            token_count: 1000,
        }],
        credits_spent: 12.5,
        platform_credits_spent: 2.5,
        total_provider_cost_in_cents: Some(3.2),
        summarized: true,
        warp_token_usage: vec![TokenUsage {
            model_id: "claude-4-7-opus-high".to_string(),
            total_tokens: 900,
            token_usage_by_category: vec![
                CategoryTokenBreakdown {
                    category: "primary_agent".to_string(),
                    tokens: 700,
                },
                CategoryTokenBreakdown {
                    category: "full_terminal_use".to_string(),
                    tokens: 200,
                },
            ],
        }],
        byok_token_usage: vec![TokenUsage {
            model_id: "gpt-5-4-high".to_string(),
            total_tokens: 300,
            token_usage_by_category: vec![CategoryTokenBreakdown {
                category: "primary_agent".to_string(),
                tokens: 300,
            }],
        }],
        total_token_cost: None,
        total_platform_cost_in_cents: None,
        tool_usage_metadata: sample_tool_usage_metadata(),
    };

    let converted: persistence::model::ConversationUsageMetadata = (&gql).into();

    assert!(converted.was_summarized);
    assert_eq!(converted.context_window_usage, 0.42);
    assert_eq!(converted.credits_spent, 12.5);
    assert_eq!(converted.platform_credits_spent, 2.5);
    assert_eq!(converted.total_provider_cost_in_cents, Some(3.2));
    assert_eq!(converted.credits_spent_for_last_block, None);

    // Token usage is sorted by model id, with warp and byok rows kept in
    // their respective buckets alongside per-category breakdowns.
    assert_eq!(converted.token_usage.len(), 2);
    let claude = &converted.token_usage[0];
    assert_eq!(claude.model_id, "claude-4-7-opus-high");
    assert_eq!(claude.warp_tokens, 900);
    assert_eq!(claude.byok_tokens, 0);
    assert_eq!(
        claude.warp_token_usage_by_category.get("primary_agent"),
        Some(&700)
    );
    assert_eq!(
        claude.warp_token_usage_by_category.get("full_terminal_use"),
        Some(&200)
    );
    let gpt = &converted.token_usage[1];
    assert_eq!(gpt.model_id, "gpt-5-4-high");
    assert_eq!(gpt.warp_tokens, 0);
    assert_eq!(gpt.byok_tokens, 300);
    assert_eq!(
        gpt.byok_token_usage_by_category.get("primary_agent"),
        Some(&300)
    );

    // Tool usage stats survive the conversion.
    let tool = &converted.tool_usage_metadata;
    assert_eq!(tool.run_command_stats.count, 4);
    assert_eq!(tool.run_command_stats.commands_executed, 3);
    assert_eq!(tool.read_files_stats.count, 7);
    assert_eq!(tool.apply_file_diff_stats.count, 5);
    assert_eq!(tool.apply_file_diff_stats.lines_added, 120);
    assert_eq!(tool.apply_file_diff_stats.lines_removed, 40);
    assert_eq!(tool.apply_file_diff_stats.files_changed, 6);

    assert_eq!(converted.context_window_segments.len(), 1);
    assert_eq!(converted.context_window_segments[0].token_count, 1000);
}

/// A single model used through both the Warp API key and a user's own key
/// merges into one row with both buckets populated.
#[test]
fn conversion_merges_warp_and_byok_usage_for_same_model() {
    let gql = ConversationUsageMetadata {
        context_window_usage: 0.0,
        context_window_segments: vec![],
        credits_spent: 0.0,
        platform_credits_spent: 0.0,
        total_provider_cost_in_cents: None,
        summarized: false,
        warp_token_usage: vec![TokenUsage {
            model_id: "claude-4-7-opus-high".to_string(),
            total_tokens: 100,
            token_usage_by_category: vec![],
        }],
        byok_token_usage: vec![TokenUsage {
            model_id: "claude-4-7-opus-high".to_string(),
            total_tokens: 50,
            token_usage_by_category: vec![],
        }],
        total_token_cost: None,
        total_platform_cost_in_cents: None,
        tool_usage_metadata: sample_tool_usage_metadata(),
    };

    let converted: persistence::model::ConversationUsageMetadata = (&gql).into();

    assert_eq!(converted.token_usage.len(), 1);
    let usage = &converted.token_usage[0];
    assert_eq!(usage.model_id, "claude-4-7-opus-high");
    assert_eq!(usage.warp_tokens, 100);
    assert_eq!(usage.byok_tokens, 50);
}

/// The restore query's aggregate `totalTokenCost`/`totalPlatformCostInCents`
/// fields (task 3.1) must convert into `ChargedUsageTotals` so the
/// conversation-details panel's historical/GraphQL-sourced path can render
/// the same "PRICING BREAKDOWN" section the live path shows.
#[test]
fn conversion_populates_charged_usage_aggregate_from_graphql_fields() {
    let gql = ConversationUsageMetadata {
        context_window_usage: 0.0,
        context_window_segments: vec![],
        credits_spent: 0.0,
        platform_credits_spent: 0.0,
        total_provider_cost_in_cents: None,
        summarized: false,
        warp_token_usage: vec![],
        byok_token_usage: vec![],
        total_token_cost: Some(TokenCostBreakdown {
            input_cost_cents: 10.0,
            output_cost_cents: 20.0,
            cache_read_cost_cents: 1.5,
            cache_write_cost_cents: 2.5,
        }),
        total_platform_cost_in_cents: Some(3.0),
        tool_usage_metadata: sample_tool_usage_metadata(),
    };

    let converted: persistence::model::ConversationUsageMetadata = (&gql).into();
    let charged_usage = converted
        .total_charged_usage
        .expect("aggregate fields should populate total_charged_usage");

    assert_eq!(charged_usage.input_cost_in_cents, 10.0);
    assert_eq!(charged_usage.output_cost_in_cents, 20.0);
    assert_eq!(charged_usage.input_cache_read_cost_in_cents, 1.5);
    assert_eq!(charged_usage.input_cache_write_cost_in_cents, 2.5);
    assert_eq!(charged_usage.platform_cost_in_cents, 3.0);
}

/// When the server omits both aggregate fields (pricing transparency
/// disabled), the conversion must produce `None` rather than a
/// coincidentally-zero `ChargedUsageTotals` -- proving the client renders
/// nothing rather than a fabricated all-zero breakdown.
#[test]
fn conversion_leaves_charged_usage_aggregate_none_when_fields_absent() {
    let gql = ConversationUsageMetadata {
        context_window_usage: 0.0,
        context_window_segments: vec![],
        credits_spent: 0.0,
        platform_credits_spent: 0.0,
        total_provider_cost_in_cents: None,
        summarized: false,
        warp_token_usage: vec![],
        byok_token_usage: vec![],
        total_token_cost: None,
        total_platform_cost_in_cents: None,
        tool_usage_metadata: sample_tool_usage_metadata(),
    };

    let converted: persistence::model::ConversationUsageMetadata = (&gql).into();

    assert_eq!(converted.total_charged_usage, None);
}

/// The conversation restore query must actually select the token-usage and
/// tool-usage fields on the wire; otherwise the conversion above would only
/// ever see empty data (the original bug).
#[test]
fn list_ai_conversations_query_selects_token_usage_fields() {
    use cynic::QueryBuilder;

    use crate::queries::list_ai_conversations::{
        ListAIConversations, ListAIConversationsInput, ListAIConversationsVariables,
    };
    use crate::request_context::{ClientContext, OsContext, RequestContext};

    let operation = ListAIConversations::build(ListAIConversationsVariables {
        input: ListAIConversationsInput {
            conversation_ids: None,
        },
        request_context: RequestContext {
            client_context: ClientContext { version: None },
            os_context: OsContext {
                category: None,
                linux_kernel_version: None,
                name: None,
                version: None,
            },
        },
    });

    for field in [
        "warpTokenUsage",
        "byokTokenUsage",
        "toolUsageMetadata",
        "tokenUsageByCategory",
    ] {
        assert!(
            operation.query.contains(field),
            "restore query no longer selects `{field}`:\n{}",
            operation.query
        );
    }
}
