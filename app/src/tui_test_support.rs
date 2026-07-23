//! Test-only app initialization used by the external `warp_tui` crate.

use ai::api_keys::ApiKeyManager;
use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use chrono::{Duration, Local};
use warp_core::SessionId;
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warpui::{AppContext, ModelContext, ModelHandle, SingletonEntity as _};

use crate::LaunchMode;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentAction, AIAgentExchangeId};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::blocklist::history_model::AIQueryHistoryOutputStatus;
use crate::ai::blocklist::local_agent_task_sync_model::LocalAgentTaskSyncModel;
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::orchestration_events::OrchestrationEventService;
use crate::ai::blocklist::{
    BlocklistAIActionModel, BlocklistAIHistoryModel, BlocklistAIPermissions, PersistedAIInput,
    PersistedAIInputType, QueuedQueryModel,
};
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::ai::connected_self_hosted_workers::ConnectedSelfHostedWorkersModel;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code_review::git_repo_model::GitRepoModels;
use crate::network::NetworkStatus;
use crate::persistence::PersistenceWriter;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::settings::manager::SettingsManager;
use crate::settings::{AISettings, PrivacySettings, init_and_register_user_preferences};
use crate::terminal::History;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::model::session::Sessions;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model_events::ModelEventDispatcher;
use crate::user_config::WarpConfig;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Builds a history model with persisted AI queries for TUI tests.
pub fn blocklist_ai_history_model_with_queries(queries: Vec<String>) -> BlocklistAIHistoryModel {
    let start_time = Local::now();
    let persisted_queries = queries
        .into_iter()
        .enumerate()
        .map(|(index, text)| PersistedAIInput {
            exchange_id: AIAgentExchangeId::new(),
            conversation_id: AIConversationId::new(),
            start_ts: start_time + Duration::milliseconds(index as i64),
            inputs: vec![PersistedAIInputType::Query {
                text,
                context: Default::default(),
                referenced_attachments: Default::default(),
            }],
            output_status: AIQueryHistoryOutputStatus::Completed,
            working_directory: None,
            model_id: LLMId::from("test-model"),
            coding_model_id: LLMId::from("test-model"),
        })
        .collect();

    BlocklistAIHistoryModel::new(persisted_queries, Vec::new(), &[])
}
/// Registers seeded command history and an active session for focused TUI history tests.
pub fn add_tui_history_test_models(
    commands: Vec<String>,
    ctx: &mut AppContext,
) -> (ModelHandle<ActiveSession>, SessionId) {
    let session_id = SessionId::from(1);
    ctx.add_singleton_model(|_| History::new_for_tui_test(session_id, commands));
    let sessions = ctx.add_model(|_| Sessions::new_for_test());
    let (_model_events_tx, model_events_rx) = async_channel::unbounded();
    let model_events =
        ctx.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    model_events.update(ctx, |dispatcher, _| {
        dispatcher.set_active_session_id(session_id);
    });
    let active_session = ctx.add_model(|ctx| ActiveSession::new(sessions, model_events, ctx));
    (active_session, session_id)
}
/// Appends a command and emits the update observed by an open TUI history menu.
pub fn append_tui_history_test_command(
    session_id: SessionId,
    command: String,
    ctx: &mut AppContext,
) {
    History::handle(ctx).update(ctx, |history, ctx| {
        history.append_command_for_tui_test(session_id, command);
        ctx.emit(crate::terminal::HistoryEvent::Updated(session_id));
    });
}

/// Queues an action as the active confirmation request for a TUI view test.
pub fn queue_tui_permission_action(
    action_model: &mut BlocklistAIActionModel,
    action: AIAgentAction,
    conversation_id: AIConversationId,
    ctx: &mut ModelContext<BlocklistAIActionModel>,
) {
    action_model.queue_confirmation_action(action, conversation_id, ctx);
}

/// Registers the app models required to construct full TUI session views in tests.
///
/// Registration order mirrors model subscription dependencies.
pub fn register_tui_session_view_test_singletons(app: &mut warpui::App) {
    app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
    app.update(warp_core::telemetry::testing::MockTelemetryContextProvider::register);
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
    app.add_singleton_model(PrivacySettings::mock);
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
    app.add_singleton_model(|_| History::default());
    app.add_singleton_model(|_| PersistenceWriter::new(None));
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
    let global_resources = crate::GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| {
        crate::GlobalResourceHandlesProvider::new(global_resources.clone())
    });

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
    app.update(crate::terminal::safe_mode_settings::SafeModeSettings::register);
    app.update(crate::terminal::session_settings::SessionSettings::register);

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
