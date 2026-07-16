//! Test-only app initialization used by the external `warp_tui` crate.

use ai::api_keys::ApiKeyManager;
use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warpui::SingletonEntity as _;

use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::blocklist::local_agent_task_sync_model::LocalAgentTaskSyncModel;
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::orchestration_events::OrchestrationEventService;
use crate::ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions, QueuedQueryModel};
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::ai::connected_self_hosted_workers::ConnectedSelfHostedWorkersModel;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::llms::LLMPreferences;
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::auth::auth_manager::AuthManager;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code_review::git_repo_model::GitRepoModels;
use crate::network::NetworkStatus;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::settings::manager::SettingsManager;
use crate::settings::{init_and_register_user_preferences, AISettings};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::user_config::WarpConfig;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::LaunchMode;

/// Registers the app models required to construct full TUI session views in tests.
///
/// Registration order mirrors model subscription dependencies.
pub fn register_tui_session_view_test_singletons(app: &mut warpui::App) {
    app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
    app.update(init_and_register_user_preferences);
    app.add_singleton_model(|_| SettingsManager::default());
    app.add_singleton_model(WarpConfig::mock);
    app.update(|ctx| {
        warpui_extras::secure_storage::register_noop("test", ctx);
    });
    app.update(AISettings::register_and_subscribe_to_events);
    CloudAgentSettings::register(app);
    app.add_singleton_model(ApiKeyManager::new);

    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|ctx| {
        let (team_client, workspace_client) = {
            let provider = ServerApiProvider::as_ref(ctx);
            (provider.get_team_client(), provider.get_workspace_client())
        };
        UserWorkspaces::mock(team_client, workspace_client, vec![], ctx)
    });
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| crate::appearance::Appearance::mock());

    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(LLMPreferences::new);
    app.add_singleton_model(HarnessAvailabilityModel::new);
    app.add_singleton_model(ConnectedSelfHostedWorkersModel::new);
    app.add_singleton_model(BlocklistAIPermissions::new);
    app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
    });
    app.add_singleton_model(|_| {
        crate::ai::document::ai_document_model::AIDocumentModel::new_for_test()
    });

    app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
    app.add_singleton_model(QueuedQueryModel::new);
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    app.add_singleton_model(OrchestrationEventService::new);
    app.add_singleton_model(LocalAgentTaskSyncModel::new);
    app.add_singleton_model(OrchestrationEventStreamer::new);
    app.add_singleton_model(|_| ActiveAgentViewsModel::new());
    app.add_singleton_model(|_| GitRepoModels::new());
    app.add_singleton_model(|ctx| {
        CodebaseIndexManager::new_for_test(ServerApiProvider::as_ref(ctx).get(), ctx)
    });
    app.add_singleton_model(AgentConversationsModel::new);

    app.add_singleton_model(crate::tui::TuiMcpManager::new_for_test);
    app.add_singleton_model(|ctx| {
        crate::changelog_model::ChangelogModel::new(ServerApiProvider::as_ref(ctx).get())
    });
    app.add_singleton_model(|_| ai::project_context::model::ProjectContextModel::default());
    app.update(crate::settings::TuiAutoupdateSettings::register);
    app.update(crate::settings::CodeSettings::register);
    app.update(crate::settings::FontSettings::register);
    app.update(crate::settings::InputSettings::register);
    app.update(crate::settings::InputModeSettings::register);
    app.update(crate::settings::SelectionSettings::register);
    app.update(crate::settings::ScrollSettings::register);
    app.update(crate::settings::EmacsBindingsSettings::register);
    app.update(crate::terminal::general_settings::GeneralSettings::register);

    app.add_singleton_model(|_| repo_metadata::repositories::DetectedRepositories::default());
    app.add_singleton_model(watcher::HomeDirectoryWatcher::new_for_test);
    app.add_singleton_model(repo_metadata::watcher::DirectoryWatcher::new);
    #[cfg(feature = "local_fs")]
    app.add_singleton_model(repo_metadata::RepoMetadataModel::new);
    app.add_singleton_model(
        crate::warp_managed_paths_watcher::WarpManagedPathsWatcher::new_for_testing,
    );
    app.add_singleton_model(crate::workflows::local_workflows::LocalWorkflows::new);
    app.add_singleton_model(crate::ai::skills::SkillManager::new);
}
