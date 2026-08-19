//! Test-only app initialization used by the external `warp_tui` crate.
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;

use ai::api_keys::ApiKeyManager;
use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use chrono::{Duration, Local};
use warp_core::SessionId;
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warpui::{AppContext, ModelContext, ModelHandle, SingletonEntity as _};

use crate::LaunchMode;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
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
use crate::ai::cloud_environments::CloudEnvironmentCatalog;
use crate::ai::connected_self_hosted_workers::ConnectedSelfHostedWorkersModel;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::ai::request_usage_model::AIRequestUsageModel;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code_review::git_repo_model::GitRepoModels;
use crate::network::NetworkStatus;
use crate::persistence::PersistenceWriter;
use crate::server::experiments::ServerExperiments;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
#[cfg(feature = "voice_input")]
use crate::server::voice_transcriber::ServerVoiceTranscriber;
use crate::settings::manager::SettingsManager;
use crate::settings::{
    AISettings, PrivacySettings, TuiVoiceSettings, init_and_register_user_preferences,
};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::event::Event;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::command_executor::NoOpCommandExecutor;
use crate::terminal::model::session::{
    BootstrapSessionType, HostInfo, IsSSHWrapperSession, Session, SessionInfo, Sessions,
};
use crate::terminal::model::terminal_model::HandlerEvent;
use crate::terminal::model_events::{AnsiHandlerEvent, ModelEvent, ModelEventDispatcher};
use crate::terminal::safe_mode_settings::SafeModeSettings;
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::shell::{Shell, ShellType};
use crate::terminal::{History, HistoryEntry, HistoryEvent};
use crate::tui_onboarding_markers::TuiOnboardingMarkers;
use crate::user_config::WarpConfig;
#[cfg(feature = "voice_input")]
use crate::voice::transcriber::VoiceTranscriber;
use crate::workspaces::team::{MembershipRole, Team, TeamMember};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;

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

/// Builds a restored conversation with one completed exchange for TUI fork tests.
pub fn forkable_tui_conversation_for_test(query: &str) -> AIConversation {
    let task_id = "tui-fork-test-root";
    let request_id = "tui-fork-test-request";
    let messages = vec![
        warp_multi_agent_api::Message {
            fetched_memories: Vec::new(),
            id: "tui-fork-test-user".to_owned(),
            task_id: task_id.to_owned(),
            server_message_data: String::new(),
            citations: Vec::new(),
            message: Some(warp_multi_agent_api::message::Message::UserQuery(
                warp_multi_agent_api::message::UserQuery {
                    query: query.to_owned(),
                    context: None,
                    referenced_attachments: HashMap::new(),
                    mode: None,
                    intended_agent: Default::default(),
                },
            )),
            request_id: request_id.to_owned(),
            timestamp: None,
        },
        warp_multi_agent_api::Message {
            fetched_memories: Vec::new(),
            id: "tui-fork-test-agent".to_owned(),
            task_id: task_id.to_owned(),
            server_message_data: String::new(),
            citations: Vec::new(),
            message: Some(warp_multi_agent_api::message::Message::AgentOutput(
                warp_multi_agent_api::message::AgentOutput {
                    text: "Original response".to_owned(),
                },
            )),
            request_id: request_id.to_owned(),
            timestamp: None,
        },
    ];
    AIConversation::new_restored(
        AIConversationId::new(),
        vec![warp_multi_agent_api::Task {
            id: task_id.to_owned(),
            messages,
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
        None,
    )
    .expect("TUI fork test conversation should restore")
}

/// Registers seeded command history and an active session for focused TUI history tests.
pub fn add_tui_history_test_models(
    commands: Vec<String>,
    ctx: &mut AppContext,
) -> (
    ModelHandle<ActiveSession>,
    SessionId,
    impl Future<Output = ()> + use<>,
) {
    let session_id = SessionId::from(1);
    let session = Arc::new(Session::new(
        SessionInfo {
            session_id,
            shell: Shell::new(ShellType::Zsh, None, None, HashSet::new(), None),
            launch_data: None,
            histfile: None,
            user: "test-user".to_owned(),
            hostname: "test-host".to_owned(),
            subshell_info: None,
            path: None,
            environment_variable_names: HashSet::new(),
            aliases: HashMap::new(),
            abbreviations: HashMap::new(),
            function_names: HashSet::new(),
            builtins: HashSet::new(),
            keywords: Vec::new(),
            is_ssh_wrapper_session: IsSSHWrapperSession::No,
            home_dir: None,
            cdpath: None,
            editor: None,
            session_type: BootstrapSessionType::Local,
            host_info: HostInfo::default(),
            wsl_name: None,
            spawning_session_id: None,
        },
        Arc::new(NoOpCommandExecutor::default()),
    ));
    let history = if ctx.has_singleton_model::<History>() {
        History::handle(ctx)
    } else {
        ctx.add_singleton_model(|_| History::default())
    };
    let (history_initialized_tx, history_initialized_rx) = async_channel::bounded(1);
    ctx.subscribe_to_model(&history, move |_, event, _| match event {
        HistoryEvent::Initialized(id) if *id == session_id => {
            let _ = history_initialized_tx.try_send(());
        }
        HistoryEvent::Initialized(_) => {}
    });
    history.update(ctx, |history, ctx| {
        history.init_session_with(session, async move { commands }, ctx);
    });

    let (executor_command_tx, _executor_command_rx) = async_channel::unbounded();
    let sessions = ctx.add_model(|ctx| Sessions::new(executor_command_tx, ctx));
    let (model_events_tx, model_events_rx) = async_channel::unbounded();
    let model_events =
        ctx.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    let (precmd_tx, precmd_rx) = async_channel::bounded(1);
    ctx.subscribe_to_model(&model_events, move |_, event, _| {
        if matches!(event, ModelEvent::Handler(AnsiHandlerEvent::Precmd)) {
            let _ = precmd_tx.try_send(());
        }
    });
    let active_session = ctx.add_model(|ctx| ActiveSession::new(sessions, model_events, ctx));
    model_events_tx
        .try_send(Event::Handler(HandlerEvent::Precmd {
            session_id: Some(session_id),
            handled_after_inband: false,
            env_vars: HashMap::new(),
        }))
        .expect("model event dispatcher should receive Precmd");
    let initialized = async move {
        history_initialized_rx
            .recv()
            .await
            .expect("history initialization should complete");
        precmd_rx
            .recv()
            .await
            .expect("Precmd should set the active session");
    };
    (active_session, session_id, initialized)
}

/// Appends a command to the history used by TUI tests.
pub fn append_tui_history_test_command(
    session_id: SessionId,
    command: String,
    ctx: &mut AppContext,
) {
    History::handle(ctx).update(ctx, |history, _| {
        let mut entry = HistoryEntry::command_only(command);
        entry.session_id = Some(session_id);
        history.append_commands(session_id, vec![entry]);
    });
}

/// Registers the production settings dependencies required by focused TUI input-mode tests.
pub fn register_tui_input_mode_test_settings(ctx: &mut AppContext) {
    if ctx.has_singleton_model::<AISettings>() {
        return;
    }
    init_and_register_user_preferences(ctx);
    ctx.add_singleton_model(|_| SettingsManager::default());
    ctx.add_singleton_model(WarpConfig::mock);
    warpui_extras::secure_storage::register_noop("test", ctx);
    ctx.add_singleton_model(|_| ServerApiProvider::new_for_test());
    ctx.add_singleton_model(|_| AuthStateProvider::new_for_test());
    AISettings::register_and_subscribe_to_events(ctx);
    ctx.add_singleton_model(|ctx| {
        let provider = ServerApiProvider::as_ref(ctx);
        UserWorkspaces::mock(
            provider.get_team_client(),
            provider.get_workspace_client(),
            Vec::new(),
            ctx,
        )
    });
}

pub fn set_tui_default_team_admin_for_test(ctx: &mut AppContext) {
    let auth = AuthStateProvider::as_ref(ctx).get();
    let user_uid = auth.user_id().expect("test user should have an id");
    let user_email = auth.user_email().expect("test user should have an email");
    let mut team = Team::from_local_cache(123.into(), "test team".to_owned(), None, None, None);
    team.members.push(TeamMember {
        uid: user_uid,
        email: user_email,
        role: MembershipRole::Owner,
    });
    let workspace = Workspace::from_local_cache(
        "workspace_uid123456789".to_owned().into(),
        "test workspace".to_owned(),
        Some(vec![team]),
    );
    let workspace_uid = workspace.uid;
    UserWorkspaces::handle(ctx).update(ctx, |workspaces, ctx| {
        workspaces.update_workspaces(vec![workspace], ctx);
        workspaces.set_current_workspace_uid(workspace_uid, ctx);
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
    app.update(TuiVoiceSettings::register);
    CloudAgentSettings::register(app);
    app.add_singleton_model(ApiKeyManager::new);

    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|_| TuiOnboardingMarkers::new_ready_for_test(false, false));
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
    app.add_singleton_model(CloudEnvironmentCatalog::new);
    app.add_singleton_model(|_| crate::appearance::Appearance::mock());

    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(LLMPreferences::new);
    app.add_singleton_model(HarnessAvailabilityModel::new);
    app.add_singleton_model(ConnectedSelfHostedWorkersModel::new);
    app.add_singleton_model(BlocklistAIPermissions::new);
    app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
    });
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
    #[cfg(feature = "voice_input")]
    {
        app.add_singleton_model(voice_input::VoiceInput::new);
        app.add_singleton_model(|ctx| {
            VoiceTranscriber::new(Arc::new(ServerVoiceTranscriber::new(
                ServerApiProvider::as_ref(ctx).get(),
            )))
        });
    }
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
    app.add_singleton_model(|ctx| ServerExperiments::new_from_cache(vec![], ctx));

    app.add_singleton_model(crate::tui::TuiMcpManager::new_for_test);
    app.add_singleton_model(crate::tui::TuiUserInfoManager::new_for_test);
    app.add_singleton_model(|ctx| {
        crate::changelog_model::ChangelogModel::new(ServerApiProvider::as_ref(ctx).get())
    });
    app.add_singleton_model(|_| ai::project_context::model::ProjectContextModel::default());
    app.update(crate::settings::TuiAutoupdateSettings::register);
    app.update(crate::settings::TuiThemeSettings::register);
    app.update(crate::settings::CodeSettings::register);
    app.update(crate::settings::FontSettings::register);
    app.update(crate::settings::InputSettings::register);
    app.update(crate::settings::InputModeSettings::register);
    app.update(crate::settings::SelectionSettings::register);
    app.update(crate::settings::ScrollSettings::register);
    app.update(crate::settings::EmacsBindingsSettings::register);
    app.update(crate::terminal::general_settings::GeneralSettings::register);
    SafeModeSettings::register(app);
    SessionSettings::register(app);

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
