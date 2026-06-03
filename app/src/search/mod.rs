pub mod action;
pub mod ai_context_menu;
mod ai_queries;
pub(crate) mod async_snapshot_data_source;
pub mod binding_source;
pub mod command_palette;
pub mod command_search;
mod env_var_collections;
pub mod external_secrets;
pub mod files;
mod filter_chip_renderer;
pub mod notebook_embedding;
mod notebooks;
mod palette_styles;
mod search_bar;
pub mod search_results_menu;
pub mod slash_command_menu;
pub mod welcome_palette;
mod workflows;

pub use data_source::QueryFilter;
use filter_chip_renderer::FilterChipRenderer;
pub use item::SearchItem;
pub use mixer::SyncDataSource;
pub use result_renderer::ItemHighlightState;
// Re-export core search types.
pub use warp_search_core::*;
pub use workflows::fuzzy_match::FuzzyMatchWorkflowResult;

pub fn query_filter_display_name(filter: QueryFilter) -> String {
    match filter {
        QueryFilter::History => i18n::t("search.filter.history"),
        QueryFilter::Workflows => i18n::t("search.filter.workflows"),
        QueryFilter::AgentModeWorkflows => i18n::t("search.filter.agent_mode_workflows"),
        QueryFilter::Notebooks => i18n::t("search.filter.notebooks"),
        QueryFilter::Plans => i18n::t("search.filter.plans"),
        QueryFilter::NaturalLanguage => i18n::t("search.filter.natural_language"),
        QueryFilter::Actions => i18n::t("search.filter.actions"),
        QueryFilter::Sessions => i18n::t("search.filter.sessions"),
        QueryFilter::Tabs => i18n::t("search.filter.tabs"),
        QueryFilter::Conversations => i18n::t("search.filter.conversations"),
        QueryFilter::LaunchConfigurations => i18n::t("search.filter.launch_configurations"),
        QueryFilter::Drive => i18n::t("search.filter.drive"),
        QueryFilter::EnvironmentVariables => i18n::t("search.filter.environment_variables"),
        QueryFilter::PromptHistory => i18n::t("search.filter.prompt_history"),
        QueryFilter::Files => i18n::t("search.filter.files"),
        QueryFilter::Commands => i18n::t("search.filter.commands"),
        QueryFilter::Blocks => i18n::t("search.filter.blocks"),
        QueryFilter::Code => i18n::t("search.filter.code"),
        QueryFilter::Rules => i18n::t("search.filter.rules"),
        QueryFilter::Repos => i18n::t("search.filter.repos"),
        QueryFilter::DiffSets => i18n::t("search.filter.diff_sets"),
        QueryFilter::StaticSlashCommands => i18n::t("search.filter.static_slash_commands"),
        QueryFilter::Skills => i18n::t("search.filter.skills"),
        QueryFilter::BaseModels => i18n::t("search.filter.base_models"),
        QueryFilter::FullTerminalUseModels => i18n::t("search.filter.full_terminal_use_models"),
        QueryFilter::CurrentDirectoryConversations => {
            i18n::t("search.filter.current_directory_conversations")
        }
    }
}

pub fn query_filter_placeholder_text(filter: QueryFilter) -> String {
    match filter {
        QueryFilter::History => i18n::t("search.placeholder.history"),
        QueryFilter::Workflows => i18n::t("search.placeholder.workflows"),
        QueryFilter::AgentModeWorkflows => i18n::t("search.placeholder.agent_mode_workflows"),
        QueryFilter::Notebooks => i18n::t("search.placeholder.notebooks"),
        QueryFilter::Plans => i18n::t("search.placeholder.plans"),
        QueryFilter::NaturalLanguage => i18n::t("search.placeholder.natural_language"),
        QueryFilter::Actions => i18n::t("search.placeholder.actions"),
        QueryFilter::Sessions => i18n::t("search.placeholder.sessions"),
        QueryFilter::Tabs => i18n::t("search.placeholder.tabs"),
        QueryFilter::Conversations => i18n::t("search.placeholder.conversations"),
        QueryFilter::LaunchConfigurations => i18n::t("search.placeholder.launch_configurations"),
        QueryFilter::Drive => i18n::t("search.placeholder.drive"),
        QueryFilter::EnvironmentVariables => i18n::t("search.placeholder.environment_variables"),
        QueryFilter::PromptHistory => i18n::t("search.placeholder.prompt_history"),
        QueryFilter::Files => i18n::t("search.placeholder.files"),
        QueryFilter::Commands => i18n::t("search.placeholder.commands"),
        QueryFilter::Blocks => i18n::t("search.placeholder.blocks"),
        QueryFilter::Code => i18n::t("search.placeholder.code"),
        QueryFilter::Rules => i18n::t("search.placeholder.rules"),
        QueryFilter::Repos => i18n::t("search.placeholder.repos"),
        QueryFilter::DiffSets => i18n::t("search.placeholder.diff_sets"),
        QueryFilter::StaticSlashCommands => i18n::t("search.placeholder.static_slash_commands"),
        QueryFilter::Skills => i18n::t("search.placeholder.skills"),
        QueryFilter::BaseModels => i18n::t("search.placeholder.base_models"),
        QueryFilter::FullTerminalUseModels => {
            i18n::t("search.placeholder.full_terminal_use_models")
        }
        QueryFilter::CurrentDirectoryConversations => {
            i18n::t("search.placeholder.current_directory_conversations")
        }
    }
}
