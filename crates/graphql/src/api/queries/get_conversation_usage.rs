use std::collections::HashMap;

use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

/*
query GetConversationUsage(
  $requestContext: RequestContext!,
  $days: Int,
  $limit: Int,
  $lastUpdatedEndTimestamp: Time
) {
  user(requestContext: $requestContext) {
    ... on UserOutput {
      user {
        conversationUsage(
          days: $days,
          limit: $limit,
          lastUpdatedEndTimestamp: $lastUpdatedEndTimestamp
        ) {
          conversationId
          title
          lastUpdated
          usageMetadata {
            contextWindowUsage
            creditsSpent
            platformCreditsSpent
            totalProviderCostInCents
            summarized
            tokenUsage { modelId totalTokens }
            warpTokenUsage { modelId totalTokens tokenUsageByCategory { category tokens } }
            byokTokenUsage { modelId totalTokens tokenUsageByCategory { category tokens } }
            totalTokenCost { inputCostCents outputCostCents cacheReadCostCents cacheWriteCostCents }
            totalPlatformCostInCents
            toolUsageMetadata {
              runCommandStats { count }
              runCommandsExecuted
              readFilesStats { count }
              searchCodebaseStats { count }
              grepStats { count }
              fileGlobStats { count }
              callMcpToolStats { count }
              readMcpResourceStats { count }
              suggestPlanStats { count }
              suggestCreatePlanStats { count }
              writeToLongRunningShellCommandStats { count }
              applyFileDiffStats { count linesAdded linesRemoved filesChanged }
              readShellCommandOutputStats { count }
              useComputerStats { count }
            }
          }
        }
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct GetConversationUsageVariables {
    pub request_context: RequestContext,
    pub days: Option<i32>,
    pub limit: Option<i32>,
    pub last_updated_end_timestamp: Option<Time>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetConversationUsageVariables"
)]
pub struct GetConversationUsage {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}
crate::client::define_operation! {
    get_conversation_usage_history(GetConversationUsageVariables) -> GetConversationUsage;
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(variables = "GetConversationUsageVariables")]
pub enum UserResult {
    UserOutput(UserOutput),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "UserOutput",
    variables = "GetConversationUsageVariables"
)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "User", variables = "GetConversationUsageVariables")]
pub struct User {
    #[arguments(
        days: $days,
        limit: $limit,
        lastUpdatedEndTimestamp: $last_updated_end_timestamp
    )]
    pub conversation_usage: Vec<ConversationUsage>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ConversationUsage {
    pub conversation_id: String,
    pub last_updated: Time,
    pub title: String,
    pub usage_metadata: ConversationUsageMetadata,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ConversationUsageMetadata {
    pub context_window_usage: f64,
    pub context_window_segments: Vec<ContextWindowSegment>,
    pub credits_spent: f64,
    pub platform_credits_spent: f64,
    pub total_provider_cost_in_cents: Option<f64>,
    pub summarized: bool,
    pub token_usage: Vec<ModelTokenUsage>,
    pub warp_token_usage: Vec<TokenUsage>,
    pub byok_token_usage: Vec<TokenUsage>,
    /// Aggregate per-token-type cost charged so far in the conversation,
    /// summed across every usage category and model. Excludes non-token
    /// costs (e.g. web search), which are tracked separately. `None` when
    /// pricing transparency is disabled server-side.
    pub total_token_cost: Option<TokenCostBreakdown>,
    /// Aggregate platform cost (in US cents) charged so far in the
    /// conversation. `None` when pricing transparency is disabled
    /// server-side.
    pub total_platform_cost_in_cents: Option<f64>,
    pub tool_usage_metadata: ToolUsageMetadata,
}

/// Aggregate, per-token-type dollar cost breakdown (in US cents), summed
/// across every usage category and model in a conversation. Excludes
/// non-token costs (e.g. web search), which are tracked separately.
#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct TokenCostBreakdown {
    pub input_cost_cents: f64,
    pub output_cost_cents: f64,
    pub cache_read_cost_cents: f64,
    pub cache_write_cost_cents: f64,
}

/// Converts the GraphQL aggregate cost-breakdown fields (task 3.1,
/// `ConversationUsageMetadata.totalTokenCost`/`totalPlatformCostInCents`)
/// into the persistence-layer `ChargedUsageTotals` shape used by the shared
/// "Tokens used" + pricing-breakdown display convention. Token counts are
/// left at zero: the aggregate GraphQL fields only carry costs, not token
/// counts, and the pricing-breakdown section only renders costs. `None`
/// when the server didn't provide either field (pricing transparency
/// disabled server-side).
pub(crate) fn charged_usage_totals_from_aggregate(
    total_token_cost: Option<&TokenCostBreakdown>,
    total_platform_cost_in_cents: Option<f64>,
) -> Option<persistence::model::ChargedUsageTotals> {
    if total_token_cost.is_none() && total_platform_cost_in_cents.is_none() {
        return None;
    }
    let mut totals = persistence::model::ChargedUsageTotals::default();
    if let Some(cost) = total_token_cost {
        totals.input_cost_in_cents = cost.input_cost_cents as f32;
        totals.output_cost_in_cents = cost.output_cost_cents as f32;
        totals.input_cache_read_cost_in_cents = cost.cache_read_cost_cents as f32;
        totals.input_cache_write_cost_in_cents = cost.cache_write_cost_cents as f32;
    }
    if let Some(platform_cost) = total_platform_cost_in_cents {
        totals.platform_cost_in_cents = platform_cost as f32;
    }
    Some(totals)
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextWindowSegmentType {
    Unknown,
    SystemPrompt,
    ToolDefinitions,
    ConversationHistory,
    LatestInput,
    Images,
    Other,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ContextWindowSegment {
    pub segment_type: ContextWindowSegmentType,
    pub token_count: i32,
}

/// Merges warp and byok per-model token usage rows into the persistence-layer
/// `ModelTokenUsage` shape, preserving per-category breakdowns. Shared by the
/// usage-history query and the conversation restore path (`crate::ai`).
pub(crate) fn convert_token_usage(
    warp_token_usage: &[TokenUsage],
    byok_token_usage: &[TokenUsage],
) -> Vec<persistence::model::ModelTokenUsage> {
    let mut usage_by_model: HashMap<String, persistence::model::ModelTokenUsage> = HashMap::new();

    for usage in warp_token_usage {
        let entry = usage_by_model
            .entry(usage.model_id.clone())
            .or_insert_with(|| persistence::model::ModelTokenUsage {
                model_id: usage.model_id.clone(),
                ..Default::default()
            });
        entry.warp_tokens += u32::try_from(usage.total_tokens).unwrap_or_default();
        for category_breakdown in &usage.token_usage_by_category {
            *entry
                .warp_token_usage_by_category
                .entry(category_breakdown.category.clone())
                .or_default() += u32::try_from(category_breakdown.tokens).unwrap_or_default();
        }
    }

    for usage in byok_token_usage {
        let entry = usage_by_model
            .entry(usage.model_id.clone())
            .or_insert_with(|| persistence::model::ModelTokenUsage {
                model_id: usage.model_id.clone(),
                ..Default::default()
            });
        entry.byok_tokens += u32::try_from(usage.total_tokens).unwrap_or_default();
        for category_breakdown in &usage.token_usage_by_category {
            *entry
                .byok_token_usage_by_category
                .entry(category_breakdown.category.clone())
                .or_default() += u32::try_from(category_breakdown.tokens).unwrap_or_default();
        }
    }

    let mut result: Vec<_> = usage_by_model.into_values().collect();
    result.sort_by(|a, b| a.model_id.cmp(&b.model_id));
    result
}

impl From<&ConversationUsageMetadata> for persistence::model::ConversationUsageMetadata {
    fn from(gql: &ConversationUsageMetadata) -> Self {
        Self {
            was_summarized: gql.summarized,
            context_window_usage: gql.context_window_usage as f32,
            credits_spent: gql.credits_spent as f32,
            platform_credits_spent: gql.platform_credits_spent as f32,
            total_provider_cost_in_cents: gql.total_provider_cost_in_cents.map(|cost| cost as f32),
            credits_spent_for_last_block: None,
            // The live per-block breakdown has no GraphQL counterpart --
            // `conversationUsage` only ever reports the conversation-wide
            // cumulative total.
            charged_usage_for_last_block: None,
            total_charged_usage: charged_usage_totals_from_aggregate(
                gql.total_token_cost.as_ref(),
                gql.total_platform_cost_in_cents,
            ),
            token_usage: convert_token_usage(&gql.warp_token_usage, &gql.byok_token_usage),
            tool_usage_metadata: (&gql.tool_usage_metadata).into(),
            context_window_segments: gql.context_window_segments.iter().map(Into::into).collect(),
        }
    }
}

impl From<ContextWindowSegmentType> for persistence::model::ContextWindowSegmentType {
    fn from(value: ContextWindowSegmentType) -> Self {
        match value {
            ContextWindowSegmentType::Unknown => Self::Unknown,
            ContextWindowSegmentType::SystemPrompt => Self::SystemPrompt,
            ContextWindowSegmentType::ToolDefinitions => Self::ToolDefinitions,
            ContextWindowSegmentType::ConversationHistory => Self::ConversationHistory,
            ContextWindowSegmentType::LatestInput => Self::LatestInput,
            ContextWindowSegmentType::Images => Self::Images,
            ContextWindowSegmentType::Other => Self::Other,
        }
    }
}

impl From<&ContextWindowSegment> for persistence::model::ContextWindowSegment {
    fn from(gql: &ContextWindowSegment) -> Self {
        Self {
            segment_type: gql.segment_type.into(),
            token_count: u32::try_from(gql.token_count).unwrap_or_default(),
        }
    }
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ModelTokenUsage {
    pub model_id: String,
    pub total_tokens: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct TokenUsage {
    pub model_id: String,
    pub total_tokens: i32,
    pub token_usage_by_category: Vec<CategoryTokenBreakdown>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct CategoryTokenBreakdown {
    pub category: String,
    pub tokens: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ToolCallStats {
    pub count: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ApplyFileDiffStats {
    pub count: i32,
    pub lines_added: i32,
    pub lines_removed: i32,
    pub files_changed: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ToolUsageMetadata {
    pub run_command_stats: ToolCallStats,
    pub run_commands_executed: i32,
    pub read_files_stats: ToolCallStats,
    pub search_codebase_stats: ToolCallStats,
    pub grep_stats: ToolCallStats,
    pub file_glob_stats: ToolCallStats,
    pub call_mcp_tool_stats: ToolCallStats,
    pub read_mcp_resource_stats: ToolCallStats,
    pub suggest_plan_stats: ToolCallStats,
    pub suggest_create_plan_stats: ToolCallStats,
    pub write_to_long_running_shell_command_stats: ToolCallStats,
    pub apply_file_diff_stats: ApplyFileDiffStats,
    pub read_shell_command_output_stats: ToolCallStats,
    pub use_computer_stats: ToolCallStats,
}

impl From<&ToolUsageMetadata> for persistence::model::ToolUsageMetadata {
    fn from(gql: &ToolUsageMetadata) -> Self {
        Self {
            run_command_stats: persistence::model::RunCommandStats {
                count: gql.run_command_stats.count,
                commands_executed: gql.run_commands_executed,
            },
            read_files_stats: persistence::model::ToolCallStats {
                count: gql.read_files_stats.count,
            },
            search_codebase_stats: persistence::model::ToolCallStats {
                count: gql.search_codebase_stats.count,
            },
            grep_stats: persistence::model::ToolCallStats {
                count: gql.grep_stats.count,
            },
            file_glob_stats: persistence::model::ToolCallStats {
                count: gql.file_glob_stats.count,
            },
            apply_file_diff_stats: persistence::model::ApplyFileDiffStats {
                count: gql.apply_file_diff_stats.count,
                lines_added: gql.apply_file_diff_stats.lines_added,
                lines_removed: gql.apply_file_diff_stats.lines_removed,
                files_changed: gql.apply_file_diff_stats.files_changed,
            },
            write_to_long_running_shell_command_stats: persistence::model::ToolCallStats {
                count: gql.write_to_long_running_shell_command_stats.count,
            },
            read_mcp_resource_stats: persistence::model::ToolCallStats {
                count: gql.read_mcp_resource_stats.count,
            },
            call_mcp_tool_stats: persistence::model::ToolCallStats {
                count: gql.call_mcp_tool_stats.count,
            },
            suggest_plan_stats: persistence::model::ToolCallStats {
                count: gql.suggest_plan_stats.count,
            },
            suggest_create_plan_stats: persistence::model::ToolCallStats {
                count: gql.suggest_create_plan_stats.count,
            },
            read_shell_command_output_stats: persistence::model::ToolCallStats {
                count: gql.read_shell_command_output_stats.count,
            },
            use_computer_stats: persistence::model::ToolCallStats {
                count: gql.use_computer_stats.count,
            },
        }
    }
}
