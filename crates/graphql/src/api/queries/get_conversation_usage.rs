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
            summarized
            tokenUsage { modelId totalTokens }
            warpTokenUsage { modelId totalTokens tokenUsageByCategory { category tokens } }
            byokTokenUsage { modelId totalTokens tokenUsageByCategory { category tokens } }
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
    pub credits_spent: f64,
    pub platform_credits_spent: f64,
    pub summarized: bool,
    pub token_usage: Vec<ModelTokenUsage>,
    pub warp_token_usage: Vec<TokenUsage>,
    pub byok_token_usage: Vec<TokenUsage>,
    pub tool_usage_metadata: ToolUsageMetadata,
}

fn convert_token_usage(
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
            credits_spent_for_last_block: None,
            token_usage: convert_token_usage(&gql.warp_token_usage, &gql.byok_token_usage),
            tool_usage_metadata: (&gql.tool_usage_metadata).into(),
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
