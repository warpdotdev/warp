use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use ai::project_context::model::ProjectContextModel;
use chrono::Utc;
use instant::Instant;
use pathfinder_geometry::rect::RectF;
use persistence::model::{
    AgentConversation, AgentConversationData, AgentConversationRecord, ConversationUsageMetadata,
};
#[cfg(feature = "local_fs")]
use repo_metadata::RepoMetadataModel;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
use serde_json::Value;
use session_sharing_protocol::common::SessionId;
use shared_session::permissions_manager::SessionPermissionsManager;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warp_core::telemetry::TelemetryEvent as _;
use warp_server_client::iap::IapManager;
use warpui::platform::{WindowBounds, WindowStyle};
use warpui::telemetry::EventPayload;
use warpui::windowing::WindowManager;
use warpui::windowing::state::ApplicationStage;
use warpui::{App, ModelHandle};
use watcher::HomeDirectoryWatcher;

use super::child_agent::restoration::is_stale_ancestor_list_completion;
use super::child_agent::{
    HiddenChildAgentConversationRequest, HiddenChildAgentTaskContext,
    create_hidden_child_agent_conversation,
};
use super::telemetry::{AgentSessionResumeTelemetryEvent, RecordedAgeBucket, ResumeOutcome};
use super::*;
use crate::ai::AIRequestUsageModel;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{
    AIAgentHarness, AIConversation, AIConversationId, ServerAIConversationMetadata,
};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier;
use crate::ai::ambient_agents::task::TaskPrincipalInfo;
use crate::ai::ambient_agents::{
    AgentSource, AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState,
};
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::history_model::CloudConversationData;
use crate::ai::blocklist::local_agent_task_sync_model::LocalAgentTaskSyncModel;
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::orchestration_events::OrchestrationEventService;
use crate::ai::blocklist::orchestration_topology::descendant_conversation_ids_in_spawn_order;
use crate::ai::blocklist::{BlocklistAIHistoryModel, QueuedQueryModel};
use crate::ai::cloud_environments::CloudEnvironmentCatalog;
use crate::ai::document::ai_document_model::AIDocumentModel;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::llms::LLMPreferences;
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::ai::mcp::{FileBasedMCPManager, FileMCPWatcher};
use crate::ai::outline::RepoOutlines;
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::ai::restored_conversations::RestoredAgentConversations;
use crate::ai::skills::SkillManager;
use crate::auth::auth_manager::AuthManager;
use crate::auth::user::TEST_USER_UID;
use crate::changelog_model::ChangelogModel;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{Owner, Revision, ServerMetadata, ServerPermissions};
use crate::context_chips::prompt::Prompt;
use crate::network::NetworkStatus;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::notebooks::manager::NotebookManager;
use crate::notebooks::notebook::NotebookView;
use crate::pricing::PricingInfoModel;
use crate::resource_center::TipsCompleted;
use crate::search::files::model::FileSearchModel;
use crate::server::cloud_objects::listener::Listener;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::ServerId;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::presigned_upload::HttpStatusError;
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::PrivacySettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::suggestions::ignored_suggestions_model::IgnoredSuggestionsModel;
use crate::system::SystemStats;
use crate::terminal::alt_screen_reporting::AltScreenReporting;
use crate::terminal::cli_agent_resume::{
    PERMISSION_POSTURE_FRESHNESS, RESUME_HISTORY_MARKER, RecordedFlag,
};
use crate::terminal::cli_agent_sessions::event::parse_event;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::terminal::event::{BlockCompletedEvent, BlockType, UserBlockCompleted};
use crate::terminal::general_settings::GeneralSettings;
use crate::terminal::history::History;
use crate::terminal::keys::TerminalKeybindings;
use crate::terminal::local_tty::TerminalManager;
use crate::terminal::local_tty::spawner::PtySpawner;
use crate::terminal::model::block::{BlockId, SerializedBlock};
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::model::terminal_model::ConversationTranscriptViewerStatus;
use crate::terminal::model_events::ModelEvent as TerminalModelEvent;
use crate::terminal::resizable_data::ResizableData;
use crate::terminal::shared_session::{
    IsSharedSessionCreator, SharedSessionActionSource, SharedSessionScrollbackType,
    SharedSessionSource, SharedSessionStatus,
};
use crate::test_util::assert_eventually;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::undo_close::UndoCloseStack;
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;
use crate::workflows::local_workflows::LocalWorkflows;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::{ActiveSession, OneTimeModalModel, WorkspaceRegistry};
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{
    AgentNotificationsModel, GlobalResourceHandles, GlobalResourceHandlesProvider, experiments,
};

fn initialize_app(app: &mut App) {
    initialize_app_with_history(app, Vec::new());
}

fn initialize_app_with_history(app: &mut App, conversations: Vec<AgentConversation>) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    // Disabled (`None`) IapManager so shared-session viewer code that reads the
    // singleton doesn't panic in tests; it is an inert no-op.
    app.add_singleton_model(|ctx| {
        IapManager::new(
            None,
            Box::new(|_| futures::FutureExt::boxed(futures::future::ready(None::<String>))),
            None,
            ctx,
        )
    });
    app.add_singleton_model(|ctx| ChangelogModel::new(ServerApiProvider::as_ref(ctx).get()));
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|_ctx| PtySpawner::new_for_test());
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(CloudEnvironmentCatalog::new);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(TeamUpdateManager::mock);
    app.add_singleton_model(Listener::mock);
    app.add_singleton_model(UpdateManager::mock);

    // Initialize file-based MCP dependencies.
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
    app.add_singleton_model(DirectoryWatcher::new);
    app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
    app.add_singleton_model(FileMCPWatcher::new);
    app.add_singleton_model(|_| FileBasedMCPManager::default());

    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(|_ctx| UserProfiles::new(Vec::new()));
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|_ctx| SyncedInputState::mock());
    app.add_singleton_model(LocalWorkflows::new);
    app.add_singleton_model(|_| Prompt::mock());
    app.add_singleton_model(|_| ResizableData::default());
    app.add_singleton_model(NotebookManager::mock);
    app.add_singleton_model(shared_session::manager::Manager::new);
    app.add_singleton_model(|_| ActiveSession::default());
    let global_resources = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resources.clone()));
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(TerminalKeybindings::new);
    app.add_singleton_model(move |_| BlocklistAIHistoryModel::new(vec![], vec![], &conversations));
    // QueuedQueryModel subscribes to history events; register after the
    // history model is in place.
    app.add_singleton_model(QueuedQueryModel::new);
    // Pill bar model subscribes to history events; register after the
    // history model is in place.
    app.add_singleton_model(|ctx| {
        crate::ai::blocklist::agent_view::orchestration_pill_bar_model::OrchestrationPillBarModel::new(
            Default::default(),
            ctx,
        )
    });
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    app.add_singleton_model(OrchestrationEventService::new);
    app.add_singleton_model(LocalAgentTaskSyncModel::new);
    app.add_singleton_model(OrchestrationEventStreamer::new);
    app.add_singleton_model(|_| ActiveAgentViewsModel::new());
    app.add_singleton_model(crate::ai::blocklist::BlocklistAIPermissions::new);
    app.add_singleton_model(AgentNotificationsModel::new);
    app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&crate::LaunchMode::new_for_unit_test(), ctx)
    });
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
    app.add_singleton_model(SessionPermissionsManager::new);
    app.add_singleton_model(LLMPreferences::new);
    app.add_singleton_model(HarnessAvailabilityModel::new);
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
    #[cfg(feature = "local_fs")]
    app.add_singleton_model(RepoMetadataModel::new);
    app.add_singleton_model(SkillManager::new);
    app.add_singleton_model(FileSearchModel::new);
    app.add_singleton_model(|_| crate::code_review::git_repo_model::GitRepoModels::new());
    app.add_singleton_model(RepoOutlines::new_for_test);
    crate::terminal::available_shells::register(app);
    app.update(experiments::init);
    AltScreenReporting::register(app);
    app.add_singleton_model(|ctx| {
        CodebaseIndexManager::new_for_test(ServerApiProvider::as_ref(ctx).get(), ctx)
    });
    app.add_singleton_model(|ctx| PersistedWorkspace::new(vec![], HashMap::new(), None, ctx));
    app.add_singleton_model(|_| ProjectContextModel::default());
    app.add_singleton_model(|ctx| crate::ai::agent_tips::AITipModel::new_for_agent_tips(ctx));
    app.add_singleton_model(|_| RestoredAgentConversations::new_seeded(vec![]));
    app.add_singleton_model(OneTimeModalModel::new);
    app.add_singleton_model(|_| WorkspaceRegistry::new());
    app.add_singleton_model(UndoCloseStack::new);
    app.add_singleton_model(|_| IgnoredSuggestionsModel::new(vec![]));
    app.add_singleton_model(|_| PricingInfoModel::new());
    app.add_singleton_model(crate::ai::pricing_promotion::PricingPromotionState::new);
    app.add_singleton_model(AIDocumentModel::new);
    app.add_singleton_model(|_| History::new(vec![]));
    app.add_singleton_model(|_| GitHubAuthNotifier::new());
    app.add_singleton_model(AgentConversationsModel::new);
    app.add_singleton_model(remote_server::manager::RemoteServerManager::new);
}

struct MockOptions {
    layout: PanesLayout,
    window_bounds: WindowBounds,
}

impl Default for MockOptions {
    fn default() -> Self {
        Self {
            layout: Default::default(),
            window_bounds: WindowBounds::ExactPosition(RectF::new(
                Vector2F::zero(),
                Vector2F::new(1024., 768.),
            )),
        }
    }
}

fn mock_pane_group(app: &mut App, options: MockOptions) -> ViewHandle<PaneGroup> {
    let tips_model = app.add_model(|_| TipsCompleted::default());
    let (_, pane_group) =
        app.add_window_with_bounds(WindowStyle::NotStealFocus, options.window_bounds, |ctx| {
            let user_default_shell_changed_banner_dismissal_model_handle =
                ctx.add_model(|_| BannerState::default());
            let block_lists = Arc::new(HashMap::new());
            PaneGroup::new_with_panes_layout(
                tips_model,
                user_default_shell_changed_banner_dismissal_model_handle,
                ServerApiProvider::as_ref(ctx).get(),
                options.layout,
                block_lists,
                AgentSessionRestore::default(),
                None,
                ctx,
            )
        });
    pane_group
}

fn get_newly_created_pane_id(panes: &PaneGroup, existing_ids: &[PaneId]) -> PaneId {
    panes
        .pane_ids()
        .find(|id| !existing_ids.contains(id))
        .unwrap()
}

fn split_pane_state(panes: &PaneGroup, pane_id: PaneId, ctx: &AppContext) -> SplitPaneState {
    panes
        .focus_state_handle()
        .as_ref(ctx)
        .split_pane_state_for(pane_id)
}

fn is_active_session(panes: &PaneGroup, pane_id: PaneId, ctx: &AppContext) -> bool {
    panes.active_session_id(ctx).map(Into::into) == Some(pane_id)
}

fn new_notebook(ctx: &mut ViewContext<PaneGroup>) -> ViewHandle<NotebookView> {
    ctx.add_typed_action_view(NotebookView::new)
}

fn new_ambient_agent_task_id() -> AmbientAgentTaskId {
    Uuid::new_v4().to_string().parse().unwrap()
}

fn ambient_agent_task_for_current_user(task_id: AmbientAgentTaskId) -> AmbientAgentTask {
    let now = Utc::now();
    AmbientAgentTask {
        task_id,
        parent_run_id: None,
        title: "Owned task".to_string(),
        state: AmbientAgentTaskState::Succeeded,
        prompt: "test".to_string(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        run_time: Some("PT1S".parse().unwrap()),
        status_message: None,
        source: Some(AgentSource::CloudMode),
        execution_location: None,
        session_id: None,
        session_link: None,
        executor: None,
        creator: Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: TEST_USER_UID.to_string(),
            display_name: None,
        }),
        conversation_id: None,
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot: None,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
        debug_agent_available: false,
        scope: None,
    }
}

/// Builds an *attachable* ambient task (InProgress + running sandbox +
/// parseable session id) so the unified child-pane dispatch resolves to
/// `AttachLive`.
fn attachable_ambient_agent_task(task_id: AmbientAgentTaskId) -> AmbientAgentTask {
    let mut task = ambient_agent_task_for_current_user(task_id);
    task.state = AmbientAgentTaskState::InProgress;
    task.is_sandbox_running = true;
    task.session_id = Some("22222222-2222-2222-2222-222222222222".to_string());
    task
}

fn mock_server_metadata() -> ServerMetadata {
    ServerMetadata {
        uid: ServerId::default(),
        revision: Revision::now(),
        metadata_last_updated_ts: Utc::now().into(),
        trashed_ts: None,
        folder_id: None,
        is_welcome_object: false,
        creator_uid: None,
        last_editor_uid: None,
        current_editor_uid: None,
    }
}

fn mock_server_permissions() -> ServerPermissions {
    ServerPermissions {
        space: Owner::mock_current_user(),
        guests: Vec::new(),
        anyone_link_sharing: None,
        permissions_last_updated_ts: Utc::now().into(),
    }
}

fn test_server_conversation_metadata(
    task_id: Option<AmbientAgentTaskId>,
) -> ServerAIConversationMetadata {
    ServerAIConversationMetadata {
        title: "Restored cloud conversation".to_string(),
        working_directory: None,
        harness: AIAgentHarness::Oz,
        usage: ConversationUsageMetadata {
            was_summarized: false,
            context_window_usage: 0.0,
            credits_spent: 0.0,
            platform_credits_spent: 0.0,
            total_provider_cost_in_cents: None,
            credits_spent_for_last_block: None,
            charged_usage_for_last_block: None,
            total_charged_usage: None,
            token_usage: vec![],
            tool_usage_metadata: Default::default(),
            context_window_segments: Vec::new(),
        },
        metadata: mock_server_metadata(),
        creator: None,
        permissions: mock_server_permissions(),
        ambient_agent_task_id: task_id,
        server_conversation_token: ServerConversationToken::new("test-server-token".to_string()),
        artifacts: Vec::new(),
    }
}

fn cloud_conversation_with_ambient_task(task_id: AmbientAgentTaskId) -> CloudConversationData {
    let mut conversation = AIConversation::new(false, false);
    conversation.set_task_id(task_id);
    conversation.set_server_metadata(test_server_conversation_metadata(Some(task_id)));
    CloudConversationData::Oz(Box::new(conversation))
}

fn persisted_remote_child_conversation(
    conversation_id: AIConversationId,
    parent_conversation_id: Option<AIConversationId>,
    parent_agent_id: Option<String>,
    task_id: AmbientAgentTaskId,
) -> AgentConversation {
    AgentConversation {
        conversation: AgentConversationRecord {
            id: 0,
            conversation_id: conversation_id.to_string(),
            conversation_data: serde_json::to_string(&AgentConversationData {
                server_conversation_token: Some("restored-child-token".to_string()),
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id,
                agent_name: Some("Agent 1".to_string()),
                orchestration_harness_type: None,
                parent_conversation_id: parent_conversation_id.map(|id| id.to_string()),
                is_remote_child: true,
                root_task_is_optimistic: None,
                run_id: Some(task_id.to_string()),
                autoexecute_override: None,
                last_event_sequence: None,
                pinned: false,
            })
            .expect("conversation data should serialize"),
            last_modified_at: Utc::now().naive_utc(),
            summary: None,
        },
        tasks: vec![warp_multi_agent_api::Task {
            id: Uuid::new_v4().to_string(),
            messages: vec![],
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
    }
}

fn start_parent_conversation(
    panes: &PaneGroup,
    parent_pane_id: PaneId,
    ctx: &mut ViewContext<PaneGroup>,
) -> AIConversationId {
    let parent_terminal_view_id = panes
        .terminal_view_from_pane_id(parent_pane_id, ctx)
        .expect("parent pane should have a terminal view")
        .id();
    start_parent_conversation_for_terminal_view(parent_terminal_view_id, ctx)
}

fn start_parent_conversation_for_terminal_view(
    terminal_view_id: EntityId,
    ctx: &mut ViewContext<PaneGroup>,
) -> AIConversationId {
    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
        history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
    })
}
fn restore_conversation_for_terminal_view(
    terminal_view_id: EntityId,
    conversation: AIConversation,
    ctx: &mut ViewContext<PaneGroup>,
) -> AIConversationId {
    let conversation_id = conversation.id();

    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
        history_model.restore_conversations(terminal_view_id, vec![conversation], ctx);
    });

    conversation_id
}

fn restore_child_conversation_for_terminal_view(
    terminal_view_id: EntityId,
    parent_conversation_id: AIConversationId,
    ctx: &mut ViewContext<PaneGroup>,
) -> AIConversationId {
    let mut child_conversation = AIConversation::new(false, false);
    child_conversation.set_parent_conversation_id(parent_conversation_id);
    restore_conversation_for_terminal_view(terminal_view_id, child_conversation, ctx)
}

fn restore_child_conversation_with_task_context_for_terminal_view(
    terminal_view_id: EntityId,
    parent_conversation_id: AIConversationId,
    task_id: AmbientAgentTaskId,
    ctx: &mut ViewContext<PaneGroup>,
) -> AIConversationId {
    let mut child_conversation = AIConversation::new(false, false);
    child_conversation.set_parent_conversation_id(parent_conversation_id);
    child_conversation.set_task_id(task_id);
    restore_conversation_for_terminal_view(terminal_view_id, child_conversation, ctx)
}

fn restore_remote_child_conversation_for_terminal_view(
    terminal_view_id: EntityId,
    parent_conversation_id: AIConversationId,
    task_id: AmbientAgentTaskId,
    ctx: &mut ViewContext<PaneGroup>,
) -> AIConversationId {
    let mut child_conversation = AIConversation::new(false, false);
    child_conversation.set_parent_conversation_id(parent_conversation_id);
    child_conversation.set_task_id(task_id);
    child_conversation.mark_as_remote_child();
    restore_conversation_for_terminal_view(terminal_view_id, child_conversation, ctx)
}
fn restore_child_conversation(
    panes: &PaneGroup,
    pane_id: PaneId,
    parent_conversation_id: AIConversationId,
    ctx: &mut ViewContext<PaneGroup>,
) -> AIConversationId {
    let terminal_view_id = panes
        .terminal_view_from_pane_id(pane_id, ctx)
        .expect("child pane should have a terminal view")
        .id();
    restore_child_conversation_for_terminal_view(terminal_view_id, parent_conversation_id, ctx)
}

fn restore_child_conversation_with_task_context(
    panes: &PaneGroup,
    pane_id: PaneId,
    parent_conversation_id: AIConversationId,
    task_id: AmbientAgentTaskId,
    ctx: &mut ViewContext<PaneGroup>,
) -> AIConversationId {
    let terminal_view_id = panes
        .terminal_view_from_pane_id(pane_id, ctx)
        .expect("child pane should have a terminal view")
        .id();
    restore_child_conversation_with_task_context_for_terminal_view(
        terminal_view_id,
        parent_conversation_id,
        task_id,
        ctx,
    )
}

fn restore_remote_child_conversation(
    panes: &PaneGroup,
    pane_id: PaneId,
    parent_conversation_id: AIConversationId,
    task_id: AmbientAgentTaskId,
    ctx: &mut ViewContext<PaneGroup>,
) -> AIConversationId {
    let terminal_view_id = panes
        .terminal_view_from_pane_id(pane_id, ctx)
        .expect("child pane should have a terminal view")
        .id();
    restore_remote_child_conversation_for_terminal_view(
        terminal_view_id,
        parent_conversation_id,
        task_id,
        ctx,
    )
}

fn enter_agent_view_for_conversation(
    panes: &PaneGroup,
    pane_id: PaneId,
    conversation_id: AIConversationId,
    ctx: &mut ViewContext<PaneGroup>,
) {
    panes
        .terminal_view_from_pane_id(pane_id, ctx)
        .expect("pane should have a terminal view")
        .update(ctx, |terminal_view, ctx| {
            terminal_view.enter_agent_view_for_conversation(
                None,
                AgentViewEntryOrigin::RestoreExistingConversation,
                conversation_id,
                ctx,
            );
        });
}

fn create_already_fullscreen_parent_pane_data(
    panes: &PaneGroup,
    ctx: &mut ViewContext<PaneGroup>,
) -> (TerminalPane, PaneId, AIConversationId) {
    let (pane_data, terminal_view) = panes.create_terminal_pane_data(
        None,
        HashMap::new(),
        IsSharedSessionCreator::No,
        None,
        None,
        ctx,
    );
    let pane_id = pane_data.terminal_pane_id().into();
    let parent_conversation_id =
        start_parent_conversation_for_terminal_view(terminal_view.id(), ctx);
    let child_conversation_id = restore_child_conversation_for_terminal_view(
        terminal_view.id(),
        parent_conversation_id,
        ctx,
    );

    terminal_view.update(ctx, |terminal_view, ctx| {
        terminal_view.enter_agent_view_for_conversation(
            None,
            AgentViewEntryOrigin::RestoreExistingConversation,
            parent_conversation_id,
            ctx,
        );
    });

    (pane_data, pane_id, child_conversation_id)
}

fn request_ambient_agent_task_id_for_hidden_child(
    panes: &PaneGroup,
    child_pane_id: PaneId,
    ctx: &mut ViewContext<PaneGroup>,
) -> Option<AmbientAgentTaskId> {
    let terminal_view = panes
        .terminal_view_from_pane_id(child_pane_id, ctx)
        .expect("child pane should have a terminal view");
    let ai_controller = terminal_view.as_ref(ctx).ai_controller().clone();

    ai_controller.update(ctx, |controller, _| controller.get_ambient_agent_task_id())
}

fn ambient_child_session_state(
    panes: &PaneGroup,
    child_pane_id: PaneId,
    ctx: &mut ViewContext<PaneGroup>,
) -> (Option<AmbientAgentTaskId>, bool, Option<AIConversationId>) {
    let terminal_view = panes
        .terminal_view_from_pane_id(child_pane_id, ctx)
        .expect("child pane should have a terminal view");
    let terminal_view_ref = terminal_view.as_ref(ctx);
    let active_conversation_id = terminal_view_ref.active_conversation_id(ctx);
    let ambient_model = terminal_view_ref
        .ambient_agent_view_model()
        .expect("child pane should have an ambient agent model")
        .as_ref(ctx);

    (
        ambient_model.task_id(),
        ambient_model.is_agent_running(),
        active_conversation_id,
    )
}

struct PreAttachReturnsFalsePane {
    pane_id: PaneId,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl PreAttachReturnsFalsePane {
    fn new(ctx: &mut ViewContext<PaneGroup>) -> Self {
        Self {
            pane_id: PaneId::dummy_pane_id(),
            pane_configuration: ctx.add_model(|_ctx| PaneConfiguration::new("")),
        }
    }
}

impl pane::PaneContent for PreAttachReturnsFalsePane {
    fn id(&self) -> PaneId {
        self.pane_id
    }

    fn pre_attach(&self, _group: &PaneGroup, _ctx: &mut ViewContext<PaneGroup>) -> bool {
        false
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        _focus_handle: focus_state::PaneFocusHandle,
        _ctx: &mut ViewContext<PaneGroup>,
    ) {
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: pane::DetachType,
        _ctx: &mut ViewContext<PaneGroup>,
    ) {
    }

    fn snapshot(&self, _app: &AppContext) -> LeafContents {
        LeafContents::GetStarted
    }

    fn has_application_focus(&self, _ctx: &mut ViewContext<PaneGroup>) -> bool {
        false
    }

    fn focus(&self, _ctx: &mut ViewContext<PaneGroup>) {}

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<pane::ShareableLink, pane::ShareableLinkError> {
        Ok(pane::ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, _ctx: &AppContext) -> bool {
        false
    }
}

// TODO: This test is commented out for now until we can fix it. It is flaky and sometimes hangs, causing the CI to cancel.
// #[test]
// #[allow(clippy::clone_on_copy)]
// fn test_pane_history() {
//     App::test((), |mut app| async move {
//         let pane_group = mock_pane_group(&mut app, platform);

//         pane_group.update(&mut app, |panes, ctx| {
//             let mut entity_ids: Vec<EntityId> =
//                 panes.view_id_to_session_data.keys().cloned().collect();

//             let first_entity_id = entity_ids.get(0).unwrap().clone();

//             // Add pane Left.
//             panes.add_pane(Direction::Left, ctx);
//             entity_ids = panes.view_id_to_session_data.keys().cloned().collect();
//             entity_ids.retain(|x| *x != first_entity_id);
//             let second_entity_id = entity_ids.get(0).unwrap().clone();
//             // Add pane Up.
//             panes.add_pane(Direction::Up, ctx);
//             entity_ids = panes.view_id_to_session_data.keys().cloned().collect();
//             entity_ids.retain(|x| *x != first_entity_id && *x != second_entity_id);
//             let third_entity_id = entity_ids.get(0).unwrap().clone();

//             assert!(panes.prev_session_id(third_entity_id).unwrap() == second_entity_id);
//         })
//     });
// }

#[test]
#[allow(clippy::clone_on_copy)]
fn test_pane_focus_on_close() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let first_pane_id = get_newly_created_pane_id(panes, &[]);

            // Add pane Left.
            panes.add_terminal_pane(Direction::Left, None, ctx);
            let second_pane_id = get_newly_created_pane_id(panes, &[first_pane_id]);

            assert!(panes.prev_pane_id(second_pane_id).unwrap() == first_pane_id);

            // Add pane Up.
            panes.add_terminal_pane(Direction::Up, None, ctx);
            let third_pane_id = get_newly_created_pane_id(panes, &[first_pane_id, second_pane_id]);

            // Close the third pane and check that the second pane opened is now focused.
            panes.close_pane(third_pane_id, ctx);
            assert_eq!(second_pane_id, panes.focused_pane_id(ctx));
        })
    });
}

#[test]
fn test_insert_hidden_child_agent_pane_keeps_focus_and_active_session() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let initial_tree_pane_count = panes.pane_count();
            let initial_content_pane_count = panes.pane_ids().count();
            let initial_visible_count = panes.visible_pane_count();
            let initial_active_session = panes.active_session_id(ctx);

            let child_pane_id = panes.insert_terminal_pane_hidden_for_child_agent(
                parent_pane_id,
                HashMap::new(),
                IsSharedSessionCreator::No,
                ctx,
            );

            assert_eq!(panes.pane_count(), initial_tree_pane_count);
            assert_eq!(panes.pane_ids().count(), initial_content_pane_count + 1);
            assert_eq!(panes.terminal_pane_ids().count(), 2);
            assert_eq!(panes.visible_pane_count(), initial_visible_count);
            assert!(panes.has_pane_id(child_pane_id.into()));
            assert!(!panes.panes.is_pane_in_tree(child_pane_id.into()));

            // The new child pane should remain off-tree and not affect visible ordering.
            assert_eq!(panes.pane_id_by_index(0), Some(parent_pane_id));
            assert_eq!(panes.pane_id_by_index(1), None);
            let visible_terminal_views = panes.visible_terminal_views(ctx);
            assert_eq!(visible_terminal_views.len(), 1);
            assert_eq!(
                visible_terminal_views[0].id(),
                panes
                    .terminal_view_from_pane_id(parent_pane_id, ctx)
                    .unwrap()
                    .id()
            );

            // Creating a hidden child pane should not steal focus or active session.
            assert_eq!(panes.focused_pane_id(ctx), parent_pane_id);
            assert_eq!(panes.active_session_id(ctx), initial_active_session);
        });
    });
}

#[test]
fn test_swapping_to_child_agent_from_maximized_pane_keeps_maximized_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            panes.add_terminal_pane(Direction::Right, None, ctx);
            panes.focus_pane(parent_pane_id, true, ctx);

            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let child = create_hidden_child_agent_conversation(
                panes,
                HiddenChildAgentConversationRequest {
                    parent_pane_id,
                    name: "Agent 1".to_string(),
                    parent_conversation_id,
                    orchestration_harness: None,
                    env_vars: HashMap::new(),
                    task_context: None,
                    is_shared_session_creator: IsSharedSessionCreator::No,
                },
                ctx,
            )
            .expect("fresh hidden child conversation should be created");
            let child_pane_id = panes
                .child_agent_panes
                .get(&child.conversation_id)
                .copied()
                .expect("fresh hidden child pane should be tracked");

            panes.toggle_maximize_pane(ctx);
            assert!(panes.is_focused_pane_maximized(ctx));

            panes.swap_active_pane_to_conversation(parent_pane_id, child.conversation_id, ctx);

            assert_eq!(panes.focused_pane_id(ctx), child_pane_id);
            assert!(panes.is_focused_pane_maximized(ctx));
            assert_eq!(
                split_pane_state(panes, child_pane_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Maximized),
            );
        });
    });
}
#[test]
fn test_insert_hidden_ambient_child_agent_pane_suppresses_details_auto_open() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let child_pane_id =
                panes.insert_ambient_agent_pane_hidden_for_child_agent(parent_pane_id, ctx);

            let terminal_view = panes
                .terminal_view_from_pane_id(child_pane_id, ctx)
                .expect("hidden ambient child pane should have a terminal view");
            assert!(
                terminal_view
                    .as_ref(ctx)
                    .is_initial_conversation_details_panel_auto_open_suppressed_for_test(),
                "hidden ambient child panes opened from the parent orchestration UI should not \
                 auto-open details during environment setup or session readiness"
            );
        });
    });
}
#[test]
fn test_hidden_child_creation_applies_ambient_task_id_to_controller() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let task_id = new_ambient_agent_task_id();

            let child = create_hidden_child_agent_conversation(
                panes,
                HiddenChildAgentConversationRequest {
                    parent_pane_id,
                    name: "Agent 1".to_string(),
                    parent_conversation_id,
                    orchestration_harness: None,
                    env_vars: HashMap::new(),
                    task_context: Some(HiddenChildAgentTaskContext {
                        task_id,
                        working_dir: None,
                    }),
                    is_shared_session_creator: IsSharedSessionCreator::No,
                },
                ctx,
            )
            .expect("fresh hidden child conversation should be created");

            let child_pane_id = panes
                .child_agent_panes
                .get(&child.conversation_id)
                .copied()
                .expect("fresh hidden child pane should be tracked");

            assert_eq!(
                request_ambient_agent_task_id_for_hidden_child(panes, child_pane_id, ctx,),
                Some(task_id)
            );
        });
    });
}

#[test]
fn test_restored_hidden_child_pane_reapplies_ambient_task_id_to_controller() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let task_id = new_ambient_agent_task_id();

            let mut child_conversation = AIConversation::new(false, false);
            child_conversation.set_parent_conversation_id(parent_conversation_id);
            child_conversation.set_task_id(task_id);
            let child_conversation_id = child_conversation.id();

            panes.create_hidden_child_agent_pane(child_conversation, parent_pane_id, ctx);

            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect("restored hidden child pane should be tracked");

            assert_eq!(
                request_ambient_agent_task_id_for_hidden_child(panes, child_pane_id, ctx,),
                Some(task_id)
            );
        });
    });
}

#[test]
fn test_restored_remote_hidden_child_pane_enters_existing_ambient_session() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let task_id = new_ambient_agent_task_id();

            // Inject an *attachable* task (InProgress + running sandbox +
            // parseable session id) so the unified dispatch resolves to
            // `AttachLive` and routes through
            // `attach_ambient_orchestration_child_session`, joining the live
            // ambient session in place.
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(attachable_ambient_agent_task(task_id));
            });

            let mut child_conversation = AIConversation::new(false, false);
            child_conversation.set_parent_conversation_id(parent_conversation_id);
            child_conversation.set_task_id(task_id);
            child_conversation.mark_as_remote_child();
            let child_conversation_id = child_conversation.id();

            panes.create_hidden_child_agent_pane(child_conversation, parent_pane_id, ctx);

            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect("restored remote hidden child pane should be tracked");

            let (ambient_task_id, is_agent_running, active_conversation_id) =
                ambient_child_session_state(panes, child_pane_id, ctx);

            assert_eq!(ambient_task_id, Some(task_id));
            assert!(
                is_agent_running,
                "remote child restore should view the existing ambient session"
            );
            assert_eq!(active_conversation_id, Some(child_conversation_id));

            // Fix B: the placeholder's local AIConversationId must remain the
            // canonical key in `child_agent_panes`. Any in-place hydration
            // (live attach, transcript merge, or fallback) must preserve this
            // key so the orchestration pill bar and topology indexes can
            // still find the pane.
            assert!(
                panes.child_agent_panes.contains_key(&child_conversation_id),
                "placeholder AIConversationId must stay the child_agent_panes key after Fix B \
                 hydration",
            );

            let terminal_view = panes
                .terminal_view_from_pane_id(child_pane_id, ctx)
                .expect("remote child pane should have a terminal view");
            assert!(
                terminal_view
                    .as_ref(ctx)
                    .is_initial_conversation_details_panel_auto_open_suppressed_for_test(),
                "remote child panes opened from the parent orchestration UI should not auto-open \
                 details when the ambient session becomes ready"
            );
        });
    });
}

/// When task data for a restored remote child is NOT yet cached at
/// `create_hidden_child_agent_pane` time, the unified dispatch resolves to
/// `Pending`: the hidden pane is still created and registered in
/// `child_agent_panes` keyed by its local AIConversationId (so the pill can
/// reveal it), using a passive loading transcript vehicle with no live attach.
/// The tracker re-drives materialization on the next lifecycle /
/// session-linked event.
#[test]
fn test_restored_remote_hidden_child_pane_pending_when_task_data_unavailable() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let task_id = new_ambient_agent_task_id();

            // Deliberately do NOT inject a task into AgentConversationsModel.
            // `get_or_async_fetch_task_data` returns `None`, so the unified
            // dispatch resolves to `Pending`: the hidden passive loading pane
            // is created and tracked so the pill can reveal it.
            // A later TasksUpdated re-drives the retained pending hydration.

            let mut child_conversation = AIConversation::new(false, false);
            child_conversation.set_parent_conversation_id(parent_conversation_id);
            child_conversation.set_task_id(task_id);
            child_conversation.mark_as_remote_child();
            let child_conversation_id = child_conversation.id();

            panes.create_hidden_child_agent_pane(child_conversation, parent_pane_id, ctx);

            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect("remote child pane must be registered even when task data unavailable");

            // The placeholder local AIConversationId remains the canonical key.
            assert!(
                panes.child_agent_panes.contains_key(&child_conversation_id),
                "placeholder AIConversationId must stay the child_agent_panes key in fallback path",
            );

            // Pending uses the passive loading presentation: no ambient
            // composer is exposed before task metadata can select live or
            // transcript materialization.
            let terminal_view = panes
                .terminal_view_from_pane_id(child_pane_id, ctx)
                .expect("pending child pane has a terminal view");
            let view = terminal_view.as_ref(ctx);
            assert!(view.ambient_agent_view_model().is_none());
            assert!(
                !view.has_agent_view_zero_state_for_test(),
                "pending child must not expose the cloud composition zero state",
            );
            assert_eq!(
                view.active_conversation_id(ctx),
                Some(child_conversation_id)
            );
            let model = view.model.lock();
            assert!(model.is_conversation_transcript_viewer());
            assert!(model.is_read_only());
            assert_eq!(
                model.conversation_transcript_viewer_status(),
                Some(&ConversationTranscriptViewerStatus::Loading),
            );
        });
    });
}

/// A terminal owner remote child (`Succeeded` run with a server
/// `conversation_id`, no live session) resolves to `LoadTranscript`: the
/// unified dispatch still materializes the hidden ambient pane keyed by the
/// placeholder's local id, into which the cloud transcript merges
/// asynchronously.
#[test]
fn test_restored_remote_hidden_child_pane_terminal_owner_loads_transcript() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let task_id = new_ambient_agent_task_id();

            // Terminal task + server conversation id -> LoadTranscript.
            let mut task = ambient_agent_task_for_current_user(task_id);
            task.state = AmbientAgentTaskState::Succeeded;
            task.is_sandbox_running = false;
            task.conversation_id = Some("owner-child-server-token".to_string());
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(task);
            });

            let mut child_conversation = AIConversation::new(false, false);
            child_conversation.set_parent_conversation_id(parent_conversation_id);
            child_conversation.set_task_id(task_id);
            child_conversation.mark_as_remote_child();
            let child_conversation_id = child_conversation.id();

            panes.create_hidden_child_agent_pane(child_conversation, parent_pane_id, ctx);

            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect("terminal owner remote child must materialize an ambient transcript pane");
            // The transcript branch builds a cloud-mode ambient pane (so the
            // pill can reveal it) keyed by the placeholder's local id.
            let (_task_id, _running, active_conversation_id) =
                ambient_child_session_state(panes, child_pane_id, ctx);
            assert_eq!(active_conversation_id, Some(child_conversation_id));
        });
    });
}

/// A terminal *viewer* child (`is_viewing_shared_session`, `Succeeded` run
/// with a server `conversation_id`, no live session) resolves to
/// `LoadTranscript` in a passive transcript pane. It must not expose the
/// ambient cloud-composition model or its new-conversation zero state while
/// the transcript fetch is in flight.
#[test]
fn test_restored_viewer_hidden_child_pane_terminal_loads_transcript() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let task_id = new_ambient_agent_task_id();

            let mut task = ambient_agent_task_for_current_user(task_id);
            task.state = AmbientAgentTaskState::Succeeded;
            task.is_sandbox_running = false;
            task.conversation_id = Some("viewer-child-server-token".to_string());
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(task);
            });

            let mut child_conversation = AIConversation::new(false, false);
            child_conversation.set_parent_conversation_id(parent_conversation_id);
            child_conversation.set_task_id(task_id);
            child_conversation.set_is_viewing_shared_session(true);
            let child_conversation_id = child_conversation.id();

            panes.create_hidden_child_agent_pane(child_conversation, parent_pane_id, ctx);

            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect("terminal viewer child must materialize a transcript pane");
            let terminal_view = panes
                .terminal_view_from_pane_id(child_pane_id, ctx)
                .expect("terminal viewer child pane has a terminal view");
            let view = terminal_view.as_ref(ctx);
            assert_eq!(
                view.active_conversation_id(ctx),
                Some(child_conversation_id),
            );
            assert!(
                view.ambient_agent_view_model().is_none(),
                "passive viewer transcripts must not retain a configuring cloud-agent model",
            );
            assert!(
                !view.has_agent_view_zero_state_for_test(),
                "viewer child placeholders must not insert new-cloud composition zero state",
            );
            let model = view.model.lock();
            assert!(model.is_conversation_transcript_viewer());
            assert!(model.is_read_only());
            assert_eq!(
                model.conversation_transcript_viewer_status(),
                Some(&ConversationTranscriptViewerStatus::Loading),
            );
        });
    });
}

#[test]
fn completed_shared_session_child_with_edit_access_uses_continuation_pane() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    let _handoff = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
    let _setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let task_id = new_ambient_agent_task_id();
            let mut task = ambient_agent_task_for_current_user(task_id);
            task.creator = Some(TaskPrincipalInfo {
                creator_type: "USER".to_string(),
                uid: "other-user".to_string(),
                display_name: None,
            });
            task.conversation_id = Some("test-server-token".to_string());
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(task);
            });

            let mut child = AIConversation::new(true, false);
            child.set_task_id(task_id);
            let child_id = child.id();
            let mut merged = child.clone();
            merged.set_server_metadata(test_server_conversation_metadata(Some(task_id)));

            let loading_pane_id = panes
                .create_child_loading_placeholder(
                    child,
                    AgentViewEntryOrigin::SharedSessionSelection,
                    ctx,
                )
                .expect("viewer child loading pane");
            panes.replace_child_loading_with_continuation_pane(
                loading_pane_id,
                child_id,
                task_id,
                merged,
                ctx,
            );

            let pane_id = panes.child_agent_panes[&child_id];
            assert_ne!(pane_id, loading_pane_id);
            let view = panes
                .terminal_view_from_pane_id(pane_id, ctx)
                .expect("continuation pane");
            assert!(view.as_ref(ctx).ambient_agent_view_model().is_some());
            let model = view.as_ref(ctx).model.lock();
            assert!(!model.is_conversation_transcript_viewer());
            assert!(!model.is_read_only());
            assert!(matches!(
                model.shared_session_status(),
                SharedSessionStatus::NotShared
            ));
        });
    });
}

#[test]
fn failed_viewer_child_session_stays_unavailable_without_retrying_same_session() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());
        let task_id = new_ambient_agent_task_id();
        let failed_session_id = SessionId::new();

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);

            let mut pending_task = ambient_agent_task_for_current_user(task_id);
            pending_task.state = AmbientAgentTaskState::Pending;
            pending_task.is_sandbox_running = false;
            pending_task.session_id = None;
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(pending_task);
            });

            let mut child_conversation = AIConversation::new(false, false);
            child_conversation.set_parent_conversation_id(parent_conversation_id);
            child_conversation.set_task_id(task_id);
            child_conversation.set_is_viewing_shared_session(true);
            let child_id = child_conversation.id();
            panes.create_hidden_child_agent_pane(child_conversation, parent_pane_id, ctx);
            let pane_id = panes.child_agent_panes[&child_id];

            panes.recover_viewer_child_join_failure(pane_id, child_id, failed_session_id, ctx);

            let mut running_task = ambient_agent_task_for_current_user(task_id);
            running_task.state = AmbientAgentTaskState::InProgress;
            running_task.is_sandbox_running = true;
            running_task.session_id = Some(failed_session_id.to_string());
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(running_task);
            });
            panes.process_pending_child_hydrations(ctx);

            assert_eq!(panes.child_agent_panes[&child_id], pane_id);
            assert_eq!(
                panes.failed_viewer_child_sessions.get(&child_id),
                Some(&failed_session_id),
            );
            assert_eq!(
                panes.pending_child_hydrations.get(&task_id),
                Some(&child_id),
            );
            let view = panes
                .terminal_view_from_pane_id(pane_id, ctx)
                .expect("pending child pane remains available");
            assert!(
                view.as_ref(ctx)
                    .is_orchestration_child_live_unavailable_for_test(),
                "failed child join should leave bounded non-error unavailable UI",
            );
        });
    });
}

/// Phase 1 integration coverage: validates that after `BlocklistAIHistoryModel`
/// restoration (the same code path the disk-load Fix C unblocks), the
/// orchestration topology is fully wired BEFORE the parent's fullscreen
/// agent view is entered, AND that entering fullscreen lazily materializes
/// the hidden child pane keyed by the placeholder local AIConversationId.
///
/// This is the integration boundary the user-visible bug lives at:
///   * pill bar / transcript name resolution must succeed before the
///     parent fullscreen entry (Fix C eagerly hydrates `conversations_by_id`
///     so this works on disk-load; this test exercises the equivalent
///     restore-into-history-model + lazy pane materialization flow).
///   * the hidden child pane must materialize in `child_agent_panes` keyed
///     by the placeholder conversation id after parent fullscreen.
///
/// The disk-load construction path (`BlocklistAIHistoryModel::new(_, _, &conversations)`
/// invoking `initialize_historical_conversations`) is covered by
/// `test_initialize_historical_conversations_eagerly_hydrates_orchestration_children`
/// in `app/src/ai/blocklist/history_model_tests.rs`. `agent_display_name_from_id`
/// resolution for restored children is covered by
/// `participant_for_restored_child_run_id_resolves_to_agent_name` in
/// `app/src/ai/blocklist/block/view_impl/orchestration_tests.rs`. The pill
/// bar data-layer coverage is in
/// `pill_bar_data_layer_finds_restored_children_before_pane_creation` in
/// `app/src/ai/blocklist/agent_view/orchestration_pill_bar_tests.rs`. This
/// test ties those three boundaries together at the PaneGroup integration
/// layer.
#[test]
fn test_pane_group_restore_loop_keeps_orchestration_topology_and_materializes_child_pane() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        let (
            parent_pane_id,
            parent_conversation_id,
            parent_run_id,
            child_conversation_id,
            child_run_id,
            child_agent_name,
        ) = pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_terminal_view_id = panes
                .terminal_view_from_pane_id(parent_pane_id, ctx)
                .expect("parent pane should have a terminal view")
                .id();

            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let parent_run_id = new_ambient_agent_task_id().to_string();
            let child_run_id = new_ambient_agent_task_id().to_string();
            let child_agent_name = "Agent 1".to_string();

            // Restore a child conversation into the parent's terminal view. This
            // is the same code path `RestoredAgentConversations::take_conversations`
            // feeds into during pane restoration. Fix C ensures the equivalent
            // wiring happens earlier (at history-model construction) so the data
            // is also available before any terminal view materializes the parent.
            let mut child_conversation = AIConversation::new(false, false);
            child_conversation.set_parent_conversation_id(parent_conversation_id);
            child_conversation.set_agent_name(child_agent_name.clone());
            let child_conversation_id = child_conversation.id();
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.restore_conversations(
                    parent_terminal_view_id,
                    vec![child_conversation],
                    ctx,
                );
                // Stamp run_ids so orchestration agent_id lookups resolve.
                history.assign_run_id_for_conversation(
                    parent_conversation_id,
                    parent_run_id.clone(),
                    None,
                    parent_terminal_view_id,
                    ctx,
                );
                history.assign_run_id_for_conversation(
                    child_conversation_id,
                    child_run_id.clone(),
                    None,
                    parent_terminal_view_id,
                    ctx,
                );
            });

            (
                parent_pane_id,
                parent_conversation_id,
                parent_run_id,
                child_conversation_id,
                child_run_id,
                child_agent_name,
            )
        });

        // BEFORE the parent's fullscreen agent view is entered, the
        // orchestration data layer must already know:
        //   (a) the parent → child topology (pill bar source),
        //   (b) the child's local conversation (with agent name set), and
        //   (c) the child's run_id → conversation id (transcript name
        //       resolution source via `conversation_id_for_agent_id`).
        pane_group.read(&app, |panes, ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);

            // (a) Topology — direct children index and the transitive walker
            // used by `OrchestrationPillBar::pill_specs` must both find the
            // child immediately, even though the hidden child pane has not
            // been created yet.
            assert_eq!(
                history.child_conversation_ids_of(&parent_conversation_id),
                &[child_conversation_id],
                "orchestration topology must list the restored child under its parent before any pane materializes",
            );
            assert_eq!(
                descendant_conversation_ids_in_spawn_order(history, parent_conversation_id),
                vec![child_conversation_id],
                "pill bar pre-order walker must reach the restored child before any pane materializes",
            );

            // (b) The child must be hydrated into `conversations_by_id`
            // with its agent name preserved — this is the data Fix C
            // eagerly populates on disk-load so the transcript name
            // resolver finds the display name instead of falling back to
            // "Unknown agent".
            let child_conversation = history
                .conversation(&child_conversation_id)
                .expect("restored child must be in conversations_by_id before parent fullscreen");
            assert_eq!(
                child_conversation.agent_name(),
                Some(child_agent_name.as_str()),
                "restored child must retain its display name for transcript / pill bar rendering",
            );

            // (c) Run-id → conversation lookups (used by `participant_for_agent_id`).
            assert_eq!(
                history.conversation_id_for_agent_id(&child_run_id),
                Some(child_conversation_id),
                "child run_id must resolve to the restored child conversation",
            );
            assert_eq!(
                history.conversation_id_for_agent_id(&parent_run_id),
                Some(parent_conversation_id),
                "parent run_id must resolve to the parent conversation",
            );

            // Hidden child pane must NOT exist yet — restoration is lazy and
            // only materializes when the parent's agent view is entered.
            assert!(
                !panes.child_agent_panes.contains_key(&child_conversation_id),
                "hidden child pane must not exist before parent fullscreen entry",
            );
        });

        // Enter the parent's fullscreen agent view. This is the trigger for
        // `restore_missing_child_agent_panes_for_parent`, which is the
        // PaneGroup-side of the user-visible restart-loop bug.
        pane_group.update(&mut app, |panes, ctx| {
            enter_agent_view_for_conversation(panes, parent_pane_id, parent_conversation_id, ctx);
        });

        // AFTER fullscreen entry, the hidden child pane must materialize in
        // `child_agent_panes` keyed by the placeholder local AIConversationId.
        pane_group.read(&app, |panes, _ctx| {
            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect("parent fullscreen entry must materialize the hidden child pane");
            assert!(
                panes.has_pane_id(child_pane_id),
                "materialized child pane must be tracked by the pane group",
            );
            assert!(
                !panes.panes.is_pane_in_tree(child_pane_id),
                "materialized child pane must remain off-tree (hidden)",
            );
        });
    });
}

/// A concurrent seed call racing a re-drive must not dispatch a second
/// `?ancestor_run_id=` request for the same parent while the first is
/// still in flight.
#[test]
fn seed_child_conversations_from_task_coalesces_concurrent_ancestor_list_fetches() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let parent_task_id = new_ambient_agent_task_id();

            // Simulate multiple entry points trying to seed the same parent
            // before its first ancestor-list fetch has resolved: a direct
            // re-entrant call, plus two `TasksUpdated` re-drives.
            panes.seed_child_conversations_from_task(parent_conversation_id, parent_task_id, ctx);
            panes.seed_child_conversations_from_task(parent_conversation_id, parent_task_id, ctx);
            panes.process_pending_parent_child_seeds(ctx);
            panes.process_pending_parent_child_seeds(ctx);

            assert_eq!(
                panes.parent_child_seed_fetch_dispatch_count, 1,
                "a parent with an ancestor-list fetch already in flight must not get a second \
                 request dispatched by a concurrent seed call or TasksUpdated re-drive",
            );
            assert!(
                panes
                    .pending_parent_child_seeds
                    .contains_key(&parent_task_id),
                "the parent should remain pending until the in-flight fetch resolves",
            );
        });
    });
}

/// Drives the real completion handler with a synthetic successful response
/// whose only child is already cached. Both the child link and the
/// pending-entry removal must happen with zero additional network dispatches.
#[test]
fn finish_seed_child_conversations_from_task_links_children_and_clears_pending_once_resolved() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let parent_task_id = new_ambient_agent_task_id();
            let child_task_id = new_ambient_agent_task_id();

            // Seed the child's task data directly so `get_or_async_fetch_task_data`
            // resolves from cache instead of issuing a network call.
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(ambient_agent_task_for_current_user(child_task_id));
            });

            // Mark pending the way `seed_child_conversations_from_task` does,
            // then drive the completion handler directly with a synthetic
            // response reporting one direct child.
            panes.seed_child_conversations_from_task(parent_conversation_id, parent_task_id, ctx);
            // The real completion callback clears `fetch_in_flight` before
            // calling `finish_seed_child_conversations_from_task`; mirror
            // that here since this test drives the completion handler
            // directly, bypassing the wrapper.
            panes
                .pending_parent_child_seeds
                .get_mut(&parent_task_id)
                .unwrap()
                .fetch_in_flight = false;
            let response = vec![ambient_agent_task_for_current_user(child_task_id)];
            panes.finish_seed_child_conversations_from_task(
                parent_conversation_id,
                parent_task_id,
                Ok(response),
                ctx,
            );

            assert!(
                !panes
                    .pending_parent_child_seeds
                    .contains_key(&parent_task_id),
                "the parent must be cleared once its only known child has resolved locally",
            );

            let history = BlocklistAIHistoryModel::as_ref(ctx);
            assert_eq!(
                history
                    .child_conversation_ids_of(&parent_conversation_id)
                    .len(),
                1,
                "the known child must be linked under the parent",
            );
        });
    });
}

/// While any reported child hasn't resolved from the local task cache yet,
/// the parent must stay pending (not be dropped) so a subsequent re-drive
/// still re-lists and can pick up a child spawned in the interim; clearing
/// early would stop discovering such children. Only once every currently
/// reported child resolves does the parent clear.
#[test]
fn finish_seed_child_conversations_from_task_stays_pending_while_a_child_is_unresolved() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let parent_task_id = new_ambient_agent_task_id();
            let unresolved_child_task_id = new_ambient_agent_task_id();

            // Deliberately do NOT insert the child's task data, so
            // `get_or_async_fetch_task_data` returns `None` for it.
            panes.seed_child_conversations_from_task(parent_conversation_id, parent_task_id, ctx);
            let response = vec![ambient_agent_task_for_current_user(
                unresolved_child_task_id,
            )];
            panes.finish_seed_child_conversations_from_task(
                parent_conversation_id,
                parent_task_id,
                Ok(response),
                ctx,
            );

            assert!(
                panes
                    .pending_parent_child_seeds
                    .contains_key(&parent_task_id),
                "the parent must remain pending while a reported child hasn't resolved yet, so \
                 the next TasksUpdated re-drive still re-lists",
            );

            // The real completion callback (in `spawn_ancestor_list_fetch_if_needed`)
            // clears `fetch_in_flight` before calling
            // `finish_seed_child_conversations_from_task`; mirror that here
            // since this test drives the completion handler directly.
            panes
                .pending_parent_child_seeds
                .get_mut(&parent_task_id)
                .unwrap()
                .fetch_in_flight = false;

            // A subsequent TasksUpdated re-drive must actually re-list (not
            // silently no-op) now that the previous fetch has completed.
            let dispatch_count_before = panes.parent_child_seed_fetch_dispatch_count;
            panes.process_pending_parent_child_seeds(ctx);
            assert_eq!(
                panes.parent_child_seed_fetch_dispatch_count,
                dispatch_count_before + 1,
                "an unresolved parent must be re-listed on the next TasksUpdated re-drive",
            );
        });
    });
}

/// A transient ancestor-list failure (e.g. a network blip) must not leave
/// the parent stranded waiting on an incidental external event that may
/// never come (e.g. an idle completed conversation) — a one-shot retry
/// must be scheduled so the fetch is retried on its own.
#[test]
fn finish_seed_child_conversations_from_task_schedules_retry_on_transient_failure() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let parent_task_id = new_ambient_agent_task_id();

            panes.seed_child_conversations_from_task(parent_conversation_id, parent_task_id, ctx);
            // Mirror the real completion callback's `fetch_in_flight` reset,
            // since this test drives the completion handler directly.
            panes
                .pending_parent_child_seeds
                .get_mut(&parent_task_id)
                .unwrap()
                .fetch_in_flight = false;
            // No `HttpStatusError` in the chain => classified as transient by
            // `is_transient_http_error` (network-level failure).
            panes.finish_seed_child_conversations_from_task(
                parent_conversation_id,
                parent_task_id,
                Err(anyhow::anyhow!("connection reset")),
                ctx,
            );

            let seed = panes
                .pending_parent_child_seeds
                .get(&parent_task_id)
                .expect("a transient failure must leave the parent pending for a retry");
            assert!(
                !seed.fetch_in_flight,
                "the completion path must not leave fetch_in_flight stuck true after handling \
                 a transient failure",
            );
            assert!(
                seed.retry_handle.is_some(),
                "a transient failure must schedule a guaranteed one-shot retry instead of \
                 relying on an incidental TasksUpdated, so no subsequent external event is \
                 needed for the parent to eventually link its children",
            );
        });
    });
}

/// If a pending seed is removed (e.g. its pane closes) and a new one
/// created for the same `parent_task_id` while the old fetch is still in
/// flight (e.g. the same parent conversation is reopened), the old
/// completion must be recognized as stale so it can't clobber the new
/// seed's in-flight state or feed it stale results.
#[test]
fn stale_ancestor_list_completion_is_detected_when_seed_removed_or_recreated() {
    let dispatched_at = Instant::now();
    let live_seed = PendingParentChildSeed {
        parent_conversation_id: AIConversationId::new(),
        fetch_in_flight: true,
        in_flight_fetch_started_at: Some(dispatched_at),
        retry_handle: None,
    };
    assert!(
        !is_stale_ancestor_list_completion(Some(&live_seed), dispatched_at),
        "a completion matching the seed's own in-flight dispatch marker must not be stale",
    );

    assert!(
        is_stale_ancestor_list_completion(None, dispatched_at),
        "a completion for a seed that was removed entirely (e.g. pane closed) must be stale",
    );

    // A later dispatch on a recreated seed (e.g. the same parent conversation
    // reopened while the old fetch was still in flight) has a distinct
    // dispatch marker.
    let recreated_seed = PendingParentChildSeed {
        parent_conversation_id: AIConversationId::new(),
        fetch_in_flight: true,
        in_flight_fetch_started_at: Some(dispatched_at + Duration::from_secs(1)),
        retry_handle: None,
    };
    assert!(
        is_stale_ancestor_list_completion(Some(&recreated_seed), dispatched_at),
        "a completion whose dispatch marker doesn't match the current seed's must be stale, \
         since a newer fetch has since been dispatched for the same parent_task_id",
    );
}

/// A permanent (non-transient) ancestor-list failure such as a 404/403
/// can't succeed by retrying blindly, so the parent must be dropped
/// instead of staying pending forever.
#[test]
fn finish_seed_child_conversations_from_task_gives_up_on_permanent_failure() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let parent_task_id = new_ambient_agent_task_id();

            panes.seed_child_conversations_from_task(parent_conversation_id, parent_task_id, ctx);
            let err = anyhow::Error::new(HttpStatusError {
                status: 404,
                body: String::new(),
            });
            panes.finish_seed_child_conversations_from_task(
                parent_conversation_id,
                parent_task_id,
                Err(err),
                ctx,
            );

            assert!(
                !panes
                    .pending_parent_child_seeds
                    .contains_key(&parent_task_id),
                "a permanent failure can't succeed by retrying blindly, so the parent must be \
                 dropped instead of staying pending forever",
            );
        });
    });
}

/// A parent with no terminal surface to seed into (e.g. a background
/// ancestor several levels above the conversation the user actually
/// opened) must not stay pending forever, since that would re-list it on
/// every future re-drive indefinitely.
#[test]
fn finish_seed_child_conversations_from_task_gives_up_when_parent_has_no_terminal_surface() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            // Never attached via `start_new_conversation` / `restore_conversations`,
            // so it has no terminal surface.
            let orphan_parent_conversation_id = AIConversationId::new();
            let parent_task_id = new_ambient_agent_task_id();
            let child_task_id = new_ambient_agent_task_id();

            // Mark pending directly (bypassing `seed_child_conversations_from_task`,
            // which would spawn a real network fetch) then drive the real
            // completion handler, so the test exercises the actual
            // no-terminal-surface early return instead of asserting against
            // fabricated state.
            panes.pending_parent_child_seeds.insert(
                parent_task_id,
                PendingParentChildSeed {
                    parent_conversation_id: orphan_parent_conversation_id,
                    fetch_in_flight: true,
                    in_flight_fetch_started_at: None,
                    retry_handle: None,
                },
            );

            let response = vec![ambient_agent_task_for_current_user(child_task_id)];
            panes.finish_seed_child_conversations_from_task(
                orphan_parent_conversation_id,
                parent_task_id,
                Ok(response),
                ctx,
            );

            assert!(
                !panes
                    .pending_parent_child_seeds
                    .contains_key(&parent_task_id),
                "a parent with no terminal surface to seed into must not stay pending forever \
                 and be re-listed on every future re-drive",
            );
        });
    });
}

#[test]
fn test_create_missing_child_agent_panes_restores_remote_child_from_history_model() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let child_conversation_id = AIConversationId::new();
            let task_id = new_ambient_agent_task_id();

            assert!(
                !panes.child_agent_panes.contains_key(&child_conversation_id),
                "child pane should not exist before startup restoration runs",
            );

            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, _| {
                history_model
                    .set_parent_for_conversation(child_conversation_id, parent_conversation_id);
            });
            RestoredAgentConversations::handle(ctx).update(ctx, |store, _| {
                *store = RestoredAgentConversations::new_seeded(vec![
                    persisted_remote_child_conversation(
                        child_conversation_id,
                        Some(parent_conversation_id),
                        None,
                        task_id,
                    ),
                ]);
            });
            // Attachable task so restoration live-attaches (was the old
            // task-data-unavailable fallback; now an explicit AttachLive).
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(attachable_ambient_agent_task(task_id));
            });

            panes.restore_missing_child_agent_panes_for_parent(
                parent_conversation_id,
                parent_pane_id,
                true,
                ctx,
            );

            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect("startup restoration should recreate the remote child pane");
            let (ambient_task_id, is_agent_running, active_conversation_id) =
                ambient_child_session_state(panes, child_pane_id, ctx);

            assert_eq!(ambient_task_id, Some(task_id));
            assert!(
                is_agent_running,
                "restored remote child pane should reconnect to the ambient session",
            );
            assert_eq!(active_conversation_id, Some(child_conversation_id));
            assert_eq!(panes.focused_pane_id(ctx), parent_pane_id);
        });
    });
}

#[test]
fn test_ambient_transcript_restore_creates_cloud_mode_pane_when_handoff_enabled() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);
    let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
    let _setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
    let _handoff = FeatureFlag::HandoffCloudCloud.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());
        let task_id = new_ambient_agent_task_id();

        pane_group.update(&mut app, |panes, ctx| {
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(ambient_agent_task_for_current_user(task_id));
            });
            panes.load_data_into_conversation_transcript_viewer(
                cloud_conversation_with_ambient_task(task_id),
                Some(task_id),
                ctx,
            );
        });

        pane_group.read(&app, |panes, ctx| {
            let terminal_view = panes
                .active_session_view(ctx)
                .expect("restored pane should have an active terminal view");
            let view = terminal_view.as_ref(ctx);
            let ambient_model = view
                .ambient_agent_view_model()
                .expect("ambient restore should create a Cloud Mode view")
                .as_ref(ctx);

            assert_eq!(ambient_model.task_id(), Some(task_id));
            assert!(ambient_model.is_agent_running());
            assert_eq!(
                view.ambient_agent_task_id_for_details_panel(ctx),
                Some(task_id)
            );
            assert!(view.active_conversation_id(ctx).is_some());

            let model = view.model.lock();
            assert!(!model.is_conversation_transcript_viewer());
            assert!(!model.is_read_only());
            assert!(matches!(
                model.shared_session_status(),
                SharedSessionStatus::NotShared
            ));
        });
    });
}

#[test]
fn test_ambient_transcript_restore_uses_generic_viewer_when_handoff_disabled() {
    let _handoff = FeatureFlag::HandoffCloudCloud.override_enabled(false);
    let _setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());
        let task_id = new_ambient_agent_task_id();

        pane_group.update(&mut app, |panes, ctx| {
            panes.load_data_into_conversation_transcript_viewer(
                cloud_conversation_with_ambient_task(task_id),
                Some(task_id),
                ctx,
            );
        });

        pane_group.read(&app, |panes, ctx| {
            let terminal_view = panes
                .active_session_view(ctx)
                .expect("fallback viewer should have an active terminal view");
            let view = terminal_view.as_ref(ctx);
            assert!(view.ambient_agent_view_model().is_none());

            let model = view.model.lock();
            assert!(model.is_conversation_transcript_viewer());
            assert!(model.is_read_only());
            assert_eq!(
                model.conversation_transcript_viewer_status(),
                Some(&ConversationTranscriptViewerStatus::ViewingAmbientConversation(task_id))
            );
        });
    });
}

/// REMOTE-2208: attaching a live execution session to a read-only conversation transcript
/// viewer is impossible (it is backed by a mock manager with no network), so the attach must
/// report failure. Reporting success left the caller focused on a transcript with no input box
/// — the "session opens but the terminal is not interactive" symptom — instead of falling back
/// to opening a fresh, writable shared-session tab.
#[test]
fn attach_execution_session_refuses_read_only_transcript_viewer_pane() {
    let _handoff = FeatureFlag::HandoffCloudCloud.override_enabled(false);
    let _setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());
        let task_id = new_ambient_agent_task_id();

        pane_group.update(&mut app, |panes, ctx| {
            panes.load_data_into_conversation_transcript_viewer(
                cloud_conversation_with_ambient_task(task_id),
                Some(task_id),
                ctx,
            );
        });

        pane_group.update(&mut app, |panes, ctx| {
            let terminal_view = panes
                .active_session_view(ctx)
                .expect("transcript viewer should have an active terminal view");
            let pane_id = panes
                .find_pane_id_for_terminal_view(terminal_view.id(), ctx)
                .expect("transcript viewer pane should be found");
            assert!(
                terminal_view.as_ref(ctx).model.lock().is_read_only(),
                "precondition: the transcript viewer pane is read-only",
            );

            assert!(
                !panes.attach_execution_session_to_ambient_pane(pane_id, SessionId::new(), ctx),
                "a read-only transcript viewer must not report a successful live-session attach",
            );
        });
    });
}

/// REMOTE-2208: the read-only state is cleared as part of reattaching, so it must only be
/// cleared when a join actually starts. A caller that gets `false` opens a fresh pane instead,
/// and clearing eagerly would leave this pane looking writable while attached to nothing.
#[test]
fn attach_execution_session_keeps_read_only_state_when_the_attach_fails() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let terminal_view = panes
                .active_session_view(ctx)
                .expect("mock pane group should have an active terminal view");
            let pane_id = panes
                .find_pane_id_for_terminal_view(terminal_view.id(), ctx)
                .expect("active terminal view should have a pane");

            // A plain terminal pane's manager is not a shared-session viewer, so the attach below
            // fails at the downcast — the same shape as a manager that is already connecting.
            terminal_view.update(ctx, |view, _| {
                view.model
                    .lock()
                    .set_shared_session_status(SharedSessionStatus::FinishedViewer);
            });
            assert!(
                terminal_view.as_ref(ctx).model.lock().is_read_only(),
                "precondition: the pane is in a finished, read-only state",
            );

            assert!(
                !panes.attach_execution_session_to_ambient_pane(pane_id, SessionId::new(), ctx),
                "precondition: this attach cannot succeed",
            );
            assert!(
                terminal_view.as_ref(ctx).model.lock().is_read_only(),
                "a failed attach must leave the pane read-only so the caller's fresh-tab fallback \
                 is not shadowed by a pane that looks writable but joined nothing",
            );
        });
    });
}

/// Pins the contract that cloud-mode shared-session viewers (the local pane
/// of a remote orchestration parent) get an `ambient_agent_view_model` so
/// the snapshot path in `TerminalPane::snapshot` can emit
/// `LeafContents::AmbientAgent` with the task id preserved. Without this,
/// the snapshot falls through to an empty `LeafContents::Terminal` and the
/// pane restores as a stray local terminal on the next launch.
#[test]
fn create_shared_session_viewer_with_cloud_mode_populates_ambient_agent_view_model() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let resources = TerminalViewResources {
                tips_completed: panes.tips_completed.clone(),
                server_api: panes.server_api.clone(),
                model_event_sender: panes.model_event_sender.clone(),
            };
            let (terminal_view, _terminal_manager) = PaneGroup::create_shared_session_viewer(
                SessionId::new(),
                resources,
                Vector2F::new(800., 600.),
                false, // enable_orchestration_polling
                true,  // is_cloud_mode
                ctx,
            );
            assert!(
                terminal_view.as_ref(ctx).ambient_agent_view_model().is_some(),
                "cloud-mode shared-session viewer must construct an ambient_agent_view_model so the snapshot path emits LeafContents::AmbientAgent on restart",
            );
        });
    });
}

/// Pins the existing behavior of the non-cloud-mode branch so callers that
/// rely on it (e.g. `new_for_shared_session_viewer`, the per-child viewer
/// path) keep getting a `TerminalView` without an `ambient_agent_view_model`.
/// Future changes that would flip this default are loud.
#[test]
fn create_shared_session_viewer_without_cloud_mode_does_not_populate_ambient_agent_view_model() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let resources = TerminalViewResources {
                tips_completed: panes.tips_completed.clone(),
                server_api: panes.server_api.clone(),
                model_event_sender: panes.model_event_sender.clone(),
            };
            let (terminal_view, _terminal_manager) = PaneGroup::create_shared_session_viewer(
                SessionId::new(),
                resources,
                Vector2F::new(800., 600.),
                false, // enable_orchestration_polling
                false, // is_cloud_mode
                ctx,
            );
            assert!(
                terminal_view.as_ref(ctx).ambient_agent_view_model().is_none(),
                "non-cloud-mode shared-session viewer must not construct an ambient_agent_view_model; existing callers depend on this",
            );
        });
    });
}

#[test]
fn test_entering_parent_agent_view_lazily_restores_hidden_child_pane() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());
        let (child_conversation_id, initial_pane_count, initial_visible_pane_count) = pane_group
            .update(&mut app, |panes, ctx| {
                let parent_pane_id = get_newly_created_pane_id(panes, &[]);
                let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
                let child_conversation_id =
                    restore_child_conversation(panes, parent_pane_id, parent_conversation_id, ctx);
                let initial_pane_count = panes.pane_count();
                let initial_visible_pane_count = panes.visible_pane_count();

                assert!(!panes.child_agent_panes.contains_key(&child_conversation_id));

                enter_agent_view_for_conversation(
                    panes,
                    parent_pane_id,
                    parent_conversation_id,
                    ctx,
                );
                (
                    child_conversation_id,
                    initial_pane_count,
                    initial_visible_pane_count,
                )
            });

        pane_group.update(&mut app, |panes, _ctx| {
            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect("parent fullscreen restore should materialize the missing child pane");

            assert!(panes.has_pane_id(child_pane_id));
            assert_eq!(panes.pane_count(), initial_pane_count);
            assert_eq!(panes.visible_pane_count(), initial_visible_pane_count);
            assert!(!panes.panes.is_pane_in_tree(child_pane_id));
        });
    });
}

#[test]
fn test_entering_remote_parent_agent_view_lazily_restores_local_hidden_child_pane() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());
        let (
            parent_pane_id,
            local_child_conversation_id,
            local_child_task_id,
            initial_pane_count,
            initial_visible_pane_count,
        ) = pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let root_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let remote_parent_task_id = new_ambient_agent_task_id();
            let remote_parent_conversation_id = restore_remote_child_conversation(
                panes,
                parent_pane_id,
                root_conversation_id,
                remote_parent_task_id,
                ctx,
            );
            let local_child_task_id = new_ambient_agent_task_id();
            let local_child_conversation_id = restore_child_conversation_with_task_context(
                panes,
                parent_pane_id,
                remote_parent_conversation_id,
                local_child_task_id,
                ctx,
            );
            let initial_pane_count = panes.pane_count();
            let initial_visible_pane_count = panes.visible_pane_count();

            assert!(
                !panes
                    .child_agent_panes
                    .contains_key(&local_child_conversation_id)
            );

            enter_agent_view_for_conversation(
                panes,
                parent_pane_id,
                remote_parent_conversation_id,
                ctx,
            );
            (
                parent_pane_id,
                local_child_conversation_id,
                local_child_task_id,
                initial_pane_count,
                initial_visible_pane_count,
            )
        });

        pane_group.update(&mut app, |panes, ctx| {
            let child_pane_id = panes
                .child_agent_panes
                .get(&local_child_conversation_id)
                .copied()
                .expect(
                    "remote parent fullscreen restore should materialize the missing local child pane",
                );

            assert!(panes.has_pane_id(child_pane_id));
            assert_eq!(panes.pane_count(), initial_pane_count);
            assert_eq!(panes.visible_pane_count(), initial_visible_pane_count);
            assert!(!panes.panes.is_pane_in_tree(child_pane_id));
            assert_eq!(panes.focused_pane_id(ctx), parent_pane_id);
            assert_eq!(
                request_ambient_agent_task_id_for_hidden_child(
                    panes,
                    child_pane_id,
                    ctx,
                ),
                Some(local_child_task_id)
            );
        });
    });
}

#[test]
fn test_entering_remote_parent_agent_view_lazily_restores_remote_hidden_child_pane() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());
        let (
            parent_pane_id,
            remote_child_conversation_id,
            remote_child_task_id,
            initial_pane_count,
            initial_visible_pane_count,
        ) = pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let root_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let remote_parent_task_id = new_ambient_agent_task_id();
            let remote_parent_conversation_id = restore_remote_child_conversation(
                panes,
                parent_pane_id,
                root_conversation_id,
                remote_parent_task_id,
                ctx,
            );
            let remote_child_task_id = new_ambient_agent_task_id();
            let remote_child_conversation_id = restore_remote_child_conversation(
                panes,
                parent_pane_id,
                remote_parent_conversation_id,
                remote_child_task_id,
                ctx,
            );
            let initial_pane_count = panes.pane_count();
            let initial_visible_pane_count = panes.visible_pane_count();

            assert!(
                !panes
                    .child_agent_panes
                    .contains_key(&remote_child_conversation_id)
            );

            // Attachable task so the lazily-restored remote child live-attaches.
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(attachable_ambient_agent_task(remote_child_task_id));
            });

            enter_agent_view_for_conversation(
                panes,
                parent_pane_id,
                remote_parent_conversation_id,
                ctx,
            );
            (
                parent_pane_id,
                remote_child_conversation_id,
                remote_child_task_id,
                initial_pane_count,
                initial_visible_pane_count,
            )
        });

        pane_group.update(&mut app, |panes, ctx| {
            let child_pane_id = panes
                .child_agent_panes
                .get(&remote_child_conversation_id)
                .copied()
                .expect(
                    "remote parent fullscreen restore should materialize the missing remote child pane",
                );
            let (ambient_task_id, is_agent_running, active_conversation_id) =
                ambient_child_session_state(panes, child_pane_id, ctx);

            assert!(panes.has_pane_id(child_pane_id));
            assert_eq!(panes.pane_count(), initial_pane_count);
            assert_eq!(panes.visible_pane_count(), initial_visible_pane_count);
            assert!(!panes.panes.is_pane_in_tree(child_pane_id));
            assert_eq!(panes.focused_pane_id(ctx), parent_pane_id);
            assert_eq!(ambient_task_id, Some(remote_child_task_id));
            assert!(
                is_agent_running,
                "remote child restore should reconnect to the existing ambient session",
            );
            assert_eq!(active_conversation_id, Some(remote_child_conversation_id));
        });
    });
}

#[test]
fn test_add_pane_restores_hidden_child_when_parent_is_already_fullscreen() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let initial_pane_count = panes.pane_count();
            let initial_visible_pane_count = panes.visible_pane_count();
            let (pane_data, parent_pane_id, child_conversation_id) =
                create_already_fullscreen_parent_pane_data(panes, ctx);

            panes.add_pane_with_direction(Direction::Right, pane_data, true, ctx);

            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect("adding an already-fullscreen parent should materialize the child pane");

            assert!(panes.has_pane_id(parent_pane_id));
            assert!(panes.has_pane_id(child_pane_id));
            assert_eq!(panes.pane_count(), initial_pane_count + 1);
            assert_eq!(panes.visible_pane_count(), initial_visible_pane_count + 1);
            assert!(!panes.panes.is_pane_in_tree(child_pane_id));
            assert_eq!(panes.focused_pane_id(ctx), parent_pane_id);
            assert_eq!(
                panes.pane_id_for_owned_conversation(child_conversation_id, ctx),
                Some(child_pane_id)
            );
        });
    });
}

#[test]
fn test_reattach_panes_restores_hidden_child_when_parent_is_already_fullscreen() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let child_conversation_id =
                restore_child_conversation(panes, parent_pane_id, parent_conversation_id, ctx);
            let initial_pane_count = panes.pane_count();
            let initial_visible_pane_count = panes.visible_pane_count();

            panes.detach_panes(ctx);
            enter_agent_view_for_conversation(panes, parent_pane_id, parent_conversation_id, ctx);
            assert!(!panes.child_agent_panes.contains_key(&child_conversation_id));

            panes.reattach_panes(ctx);

            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect(
                    "reattaching an already-fullscreen parent should materialize the child pane",
                );

            assert!(panes.has_pane_id(child_pane_id));
            assert_eq!(panes.pane_count(), initial_pane_count);
            assert_eq!(panes.visible_pane_count(), initial_visible_pane_count);
            assert!(!panes.panes.is_pane_in_tree(child_pane_id));
            assert_eq!(
                panes.pane_id_for_owned_conversation(child_conversation_id, ctx),
                Some(child_pane_id)
            );
        });
    });
}

#[test]
fn test_restore_closed_pane_restores_hidden_child_when_parent_is_already_fullscreen() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);
    let _undo_closed_panes = FeatureFlag::UndoClosedPanes.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            panes.add_pane_with_direction(
                Direction::Right,
                NotebookPane::new(new_notebook(ctx), ctx),
                false,
                ctx,
            );

            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let child_conversation_id =
                restore_child_conversation(panes, parent_pane_id, parent_conversation_id, ctx);
            let initial_pane_count = panes.pane_count();
            let initial_visible_pane_count = panes.visible_pane_count();

            panes.close_pane(parent_pane_id, ctx);
            assert!(panes.is_pane_hidden_for_close(parent_pane_id));

            enter_agent_view_for_conversation(panes, parent_pane_id, parent_conversation_id, ctx);
            assert!(!panes.child_agent_panes.contains_key(&child_conversation_id));

            assert!(panes.restore_closed_pane(parent_pane_id, ctx));

            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect(
                    "restoring an already-fullscreen closed parent should materialize the child pane",
                );

            assert!(panes.has_pane_id(child_pane_id));
            assert_eq!(panes.pane_count(), initial_pane_count);
            assert_eq!(panes.visible_pane_count(), initial_visible_pane_count);
            assert!(!panes.panes.is_pane_in_tree(child_pane_id));
            assert_eq!(panes.focused_pane_id(ctx), parent_pane_id);
            assert_eq!(
                panes.pane_id_for_owned_conversation(child_conversation_id, ctx),
                Some(child_pane_id)
            );
        });
    });
}

#[test]
fn test_replace_pane_restores_hidden_child_when_replacement_is_already_fullscreen() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let original_pane_id = get_newly_created_pane_id(panes, &[]);
            let initial_pane_count = panes.pane_count();
            let initial_visible_pane_count = panes.visible_pane_count();
            let (replacement_pane, replacement_pane_id, child_conversation_id) =
                create_already_fullscreen_parent_pane_data(panes, ctx);

            assert!(panes.replace_pane(original_pane_id, replacement_pane, false, ctx));

            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect(
                    "replacing with an already-fullscreen parent should materialize the child pane",
                );

            assert!(!panes.has_pane_id(original_pane_id));
            assert!(panes.has_pane_id(replacement_pane_id));
            assert!(panes.has_pane_id(child_pane_id));
            assert_eq!(panes.pane_count(), initial_pane_count);
            assert_eq!(panes.visible_pane_count(), initial_visible_pane_count);
            assert!(!panes.panes.is_pane_in_tree(child_pane_id));
            assert_eq!(panes.focused_pane_id(ctx), replacement_pane_id);
            assert_eq!(
                panes.pane_id_for_owned_conversation(child_conversation_id, ctx),
                Some(child_pane_id)
            );
        });
    });
}

#[test]
fn test_ensure_hidden_child_agent_pane_materializes_missing_child_pane() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let child_conversation_id =
                restore_child_conversation(panes, parent_pane_id, parent_conversation_id, ctx);
            let initial_pane_count = panes.pane_count();

            assert!(!panes.child_agent_panes.contains_key(&child_conversation_id));
            assert!(
                panes.ensure_hidden_child_agent_pane_for_conversation(child_conversation_id, ctx),
                "navigation fallback should materialize the missing child pane on demand"
            );

            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect("on-demand ensure should track the restored child pane");
            assert!(panes.has_pane_id(child_pane_id));
            assert_eq!(panes.pane_count(), initial_pane_count);
            assert!(!panes.panes.is_pane_in_tree(child_pane_id));
        });
    });
}

#[test]
fn test_ensure_hidden_child_agent_pane_materializes_restored_remote_child_linked_by_parent_agent_id()
 {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_terminal_view_id = panes
                .terminal_view_from_pane_id(parent_pane_id, ctx)
                .expect("parent pane should have a terminal view")
                .id();
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let child_conversation_id = AIConversationId::new();
            let parent_run_id = new_ambient_agent_task_id().to_string();
            let task_id = new_ambient_agent_task_id();
            let initial_pane_count = panes.pane_count();

            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model.assign_run_id_for_conversation(
                    parent_conversation_id,
                    parent_run_id.clone(),
                    None,
                    parent_terminal_view_id,
                    ctx,
                );
                history_model
                    .set_parent_for_conversation(child_conversation_id, parent_conversation_id);
            });
            RestoredAgentConversations::handle(ctx).update(ctx, |store, _| {
                *store = RestoredAgentConversations::new_seeded(vec![
                    persisted_remote_child_conversation(
                        child_conversation_id,
                        None,
                        Some(parent_run_id),
                        task_id,
                    ),
                ]);
            });
            // Attachable task so the on-demand restore live-attaches.
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(attachable_ambient_agent_task(task_id));
            });

            assert!(!panes.child_agent_panes.contains_key(&child_conversation_id));
            assert!(
                panes.ensure_hidden_child_agent_pane_for_conversation(child_conversation_id, ctx),
                "navigation fallback should restore a parent_agent_id-linked remote child pane",
            );

            let child_pane_id = panes
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .expect("parent_agent_id-linked child pane should be tracked after restoration");
            let (ambient_task_id, is_agent_running, active_conversation_id) =
                ambient_child_session_state(panes, child_pane_id, ctx);

            assert!(panes.has_pane_id(child_pane_id));
            assert_eq!(panes.pane_count(), initial_pane_count);
            assert!(!panes.panes.is_pane_in_tree(child_pane_id));
            assert_eq!(ambient_task_id, Some(task_id));
            assert!(
                is_agent_running,
                "restored remote child pane should reconnect to the ambient session",
            );
            assert_eq!(active_conversation_id, Some(child_conversation_id));
        });
    });
}

#[test]
fn test_entering_parent_agent_view_skips_child_owned_by_another_pane() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());
        let (child_conversation_id, initial_pane_count) =
            pane_group.update(&mut app, |panes, ctx| {
                let parent_pane_id = get_newly_created_pane_id(panes, &[]);
                let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);

                panes.add_terminal_pane(Direction::Right, None, ctx);
                let sibling_pane_id = get_newly_created_pane_id(panes, &[parent_pane_id]);
                let child_conversation_id =
                    restore_child_conversation(panes, sibling_pane_id, parent_conversation_id, ctx);
                let initial_pane_count = panes.pane_count();

                enter_agent_view_for_conversation(
                    panes,
                    sibling_pane_id,
                    child_conversation_id,
                    ctx,
                );
                assert_eq!(
                    panes.pane_id_for_owned_conversation(child_conversation_id, ctx),
                    Some(sibling_pane_id)
                );

                enter_agent_view_for_conversation(
                    panes,
                    parent_pane_id,
                    parent_conversation_id,
                    ctx,
                );
                (child_conversation_id, initial_pane_count)
            });

        pane_group.update(&mut app, |panes, _ctx| {
            assert!(!panes.child_agent_panes.contains_key(&child_conversation_id));
            assert_eq!(panes.pane_count(), initial_pane_count);
        });
    });
}

#[test]
fn test_ensure_hidden_child_agent_pane_skips_child_owned_by_another_pane_group() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let parent_pane_group = mock_pane_group(&mut app, Default::default());
        let other_pane_group = mock_pane_group(&mut app, Default::default());

        let parent_conversation_id = parent_pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            start_parent_conversation(panes, parent_pane_id, ctx)
        });
        let (child_conversation_id, child_owner_terminal_view_id) =
            other_pane_group.update(&mut app, |panes, ctx| {
                let child_pane_id = get_newly_created_pane_id(panes, &[]);
                let child_conversation_id =
                    restore_child_conversation(panes, child_pane_id, parent_conversation_id, ctx);
                let initial_owner_terminal_view_id = panes
                    .terminal_view_from_pane_id(child_pane_id, ctx)
                    .expect("child pane should have a terminal view")
                    .id();

                enter_agent_view_for_conversation(panes, child_pane_id, child_conversation_id, ctx);
                (child_conversation_id, initial_owner_terminal_view_id)
            });

        parent_pane_group.update(&mut app, |panes, ctx| {
            let initial_pane_count = panes.pane_count();

            assert!(
                panes.ensure_hidden_child_agent_pane_for_conversation(child_conversation_id, ctx),
                "cross-tab child ownership should be treated as already reachable"
            );
            assert!(!panes.child_agent_panes.contains_key(&child_conversation_id));
            assert_eq!(panes.pane_count(), initial_pane_count);
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .terminal_surface_id_for_conversation(&child_conversation_id),
                Some(child_owner_terminal_view_id)
            );
        });
    });
}

#[test]
fn test_entering_parent_agent_view_skips_child_owned_by_another_pane_group() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let parent_pane_group = mock_pane_group(&mut app, Default::default());
        let other_pane_group = mock_pane_group(&mut app, Default::default());

        let (parent_conversation_id, parent_pane_id) =
            parent_pane_group.update(&mut app, |panes, ctx| {
                let parent_pane_id = get_newly_created_pane_id(panes, &[]);
                let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
                (parent_conversation_id, parent_pane_id)
            });
        let (child_conversation_id, child_owner_terminal_view_id) =
            other_pane_group.update(&mut app, |panes, ctx| {
                let child_pane_id = get_newly_created_pane_id(panes, &[]);
                let child_conversation_id =
                    restore_child_conversation(panes, child_pane_id, parent_conversation_id, ctx);
                let initial_owner_terminal_view_id = panes
                    .terminal_view_from_pane_id(child_pane_id, ctx)
                    .expect("child pane should have a terminal view")
                    .id();

                enter_agent_view_for_conversation(panes, child_pane_id, child_conversation_id, ctx);
                (child_conversation_id, initial_owner_terminal_view_id)
            });
        let initial_pane_count = parent_pane_group.update(&mut app, |panes, ctx| {
            let initial_pane_count = panes.pane_count();
            enter_agent_view_for_conversation(panes, parent_pane_id, parent_conversation_id, ctx);
            initial_pane_count
        });

        parent_pane_group.update(&mut app, |panes, ctx| {
            assert!(!panes.child_agent_panes.contains_key(&child_conversation_id));
            assert_eq!(panes.pane_count(), initial_pane_count);
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .terminal_surface_id_for_conversation(&child_conversation_id),
                Some(child_owner_terminal_view_id)
            );
        });
    });
}

#[test]
fn test_active_session_id_reset_on_last_pane_close() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let terminal_id = get_newly_created_pane_id(panes, &[]);
            assert_eq!(
                panes.active_session_id(ctx),
                terminal_id.as_terminal_pane_id()
            );

            // Add a non-terminal pane (Notebook) so the pane group remains alive when terminal is closed.
            panes.add_pane_with_direction(
                Direction::Right,
                NotebookPane::new(new_notebook(ctx), ctx),
                false, /* focus_new_pane */
                ctx,
            );

            // Close the terminal.
            panes.close_pane(terminal_id, ctx);

            // active_session_id should be None after closing the last pane.
            assert_eq!(
                panes.active_session_id(ctx),
                None,
                "active_session_id should be None after closing the last pane"
            );
        });
    });
}

#[test]
fn test_close_last_pane_clears_share_modal_state() {
    let _undo_closed_panes = FeatureFlag::UndoClosedPanes.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let pane_id = get_newly_created_pane_id(panes, &[]);
            panes.terminal_with_open_share_block_modal = Some(
                pane_id
                    .as_terminal_pane_id()
                    .expect("newly created pane should be a terminal"),
            );

            panes.close_pane(pane_id, ctx);

            assert_eq!(panes.terminal_with_open_share_block_modal, None);
        });
    });
}

#[test]
fn test_add_pane_aborts_cleanly_when_pre_attach_returns_false() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let before_snapshot = panes.snapshot(ctx);
            let before_count = panes.pane_count();

            panes.add_pane_with_direction(
                Direction::Right,
                PreAttachReturnsFalsePane::new(ctx),
                true, /* focus_new_pane */
                ctx,
            );

            assert_eq!(panes.pane_count(), before_count);
            assert_eq!(panes.snapshot(ctx), before_snapshot);
        });
    });
}

#[test]
fn test_focus_notebook() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let first_terminal_id = get_newly_created_pane_id(panes, &[]);

            // Add a notebook to the left.
            panes.add_pane_with_direction(
                Direction::Left,
                NotebookPane::new(new_notebook(ctx), ctx),
                true, /* focus_new_pane */
                ctx,
            );
            let notebook_id = get_newly_created_pane_id(panes, &[first_terminal_id]);

            // The new pane should be focused, but the terminal is still the active session.
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(
                panes.active_session_id(ctx).map(Into::into),
                Some(first_terminal_id)
            );
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert!(is_active_session(panes, first_terminal_id, ctx));
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Focused)
            );

            // Add a terminal below.
            panes.add_terminal_pane(Direction::Down, None, ctx);
            let second_terminal_id =
                get_newly_created_pane_id(panes, &[first_terminal_id, notebook_id]);

            // The new terminal should be both focused and the active session.
            assert_eq!(panes.focused_pane_id(ctx), second_terminal_id);
            assert_eq!(
                panes.active_session_id(ctx).map(Into::into),
                Some(second_terminal_id)
            );
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert!(!is_active_session(panes, first_terminal_id, ctx));
            assert_eq!(
                split_pane_state(panes, second_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Focused)
            );
            assert!(is_active_session(panes, second_terminal_id, ctx));
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );

            // Close the new terminal. Focus should switch to the notebook, and the first terminal
            // session will activate.
            panes.close_pane(second_terminal_id, ctx);
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(
                panes.active_session_id(ctx).map(Into::into),
                Some(first_terminal_id)
            );
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Focused)
            );
            assert!(is_active_session(panes, first_terminal_id, ctx));
        })
    });
}

#[test]
fn test_group_without_terminals() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let terminal_id = get_newly_created_pane_id(panes, &[]);

            // Add a notebook to the left.
            panes.add_pane_with_direction(
                Direction::Left,
                NotebookPane::new(new_notebook(ctx), ctx),
                true, /* focus_new_pane */
                ctx,
            );
            let notebook_id = get_newly_created_pane_id(panes, &[terminal_id]);

            // Close the terminal, which should leave the group without an active session.
            panes.close_pane(terminal_id, ctx);
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(panes.active_session_id(ctx), None);
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::NotInSplitPane
            );
        });
    });
}

#[test]
fn test_close_active_session() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            // Add two terminal sessions.
            let first_terminal_id = get_newly_created_pane_id(panes, &[]);
            panes.add_terminal_pane(Direction::Up, None, ctx);
            let second_terminal_id = get_newly_created_pane_id(panes, &[first_terminal_id]);

            // Add a notebook to the left.
            panes.add_pane_with_direction(
                Direction::Left,
                NotebookPane::new(new_notebook(ctx), ctx),
                true, /* focus_new_pane */
                ctx,
            );
            let notebook_id =
                get_newly_created_pane_id(panes, &[first_terminal_id, second_terminal_id]);
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(
                panes.active_session_id(ctx).map(Into::into),
                Some(second_terminal_id)
            );

            // Close the active session, which should leave the notebook focused and activate the
            // remaining session.
            panes.close_pane(second_terminal_id, ctx);
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(
                panes.active_session_id(ctx).map(Into::into),
                Some(first_terminal_id)
            );
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert!(is_active_session(panes, first_terminal_id, ctx));

            // Now, focus the remaining session, which should keep it activated.
            panes.focus_pane_by_id(first_terminal_id, ctx);
            assert_eq!(panes.focused_pane_id(ctx), first_terminal_id);
            assert_eq!(
                panes.active_session_id(ctx).map(Into::into),
                Some(first_terminal_id)
            );
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Focused)
            );
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert!(is_active_session(panes, first_terminal_id, ctx));
        });
    });
}

#[test]
fn test_update_session_visibility() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let pane_group = mock_pane_group(&mut app, Default::default());
        pane_group.update(&mut app, |panes, ctx| {
            // Assert that there is no active window.
            WindowManager::handle(ctx).read(ctx, |state, _| {
                assert_eq!(state.stage(), ApplicationStage::Starting);
                assert!(state.active_window().is_none());
            });

            fn visibility_matches(panes: &PaneGroup, expected: bool, ctx: &ViewContext<PaneGroup>) {
                for data in panes.panes_of::<TerminalPane>() {
                    let view = data.terminal_view(ctx).as_ref(ctx);
                    assert_eq!(
                        view.was_ever_visible(),
                        expected,
                        "View {} visibility was {}, expected {}",
                        data.terminal_view(ctx).id(),
                        view.was_ever_visible(),
                        expected
                    );
                }
            }

            // Add pane Left.
            panes.add_terminal_pane(Direction::Left, None, ctx);

            // Assert that neither of the panes are marked as visible (due
            // to the fact that the window is not active).
            visibility_matches(panes, false, ctx);

            let window_id = ctx.window_id();
            WindowManager::handle(ctx).update(ctx, |state, ctx| {
                state.overwrite_for_test(ApplicationStage::Active, Some(window_id));
                ctx.notify();
            });

            // Assert that both of the panes are still not marked as
            // visible, given the fact that the pane group is not focused.
            visibility_matches(panes, false, ctx);

            panes.focus(ctx);

            // Assert that both of the panes are now visible.
            visibility_matches(panes, true, ctx);
        })
    });
}

#[test]
fn test_initial_widths_are_computed_correctly() {
    use launch_config::PaneTemplateType::*;

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Define a simple macro to help us create new leaf panes.
        macro_rules! leaf_pane {
            () => {
                PaneTemplate {
                    is_focused: None,
                    cwd: "".into(),
                    commands: vec![],
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                }
            };
        }

        // Pick an arbitrary initial window that isn't the same as the
        // fallback value.
        let window_width = 864.;
        let window_height = 636.;
        assert_ne!(window_width, FALLBACK_INITIAL_WINDOW_SIZE.x());
        assert_ne!(window_height, FALLBACK_INITIAL_WINDOW_SIZE.y());

        // Create a template that looks like the following, with each pane
        // numbered by its index in the pane group:
        //
        //  ---------------------
        //  |         0         |
        //  | __________________|
        //  |     1   |____2____|
        //  | ________|____3____|
        //  |   4  |   5  |  6  |
        //  |      |      |     |
        //  ---------------------
        let template = PaneBranchTemplate {
            split_direction: launch_config::SplitDirection::Vertical,
            panes: vec![
                leaf_pane!(),
                PaneBranchTemplate {
                    split_direction: launch_config::SplitDirection::Horizontal,
                    panes: vec![
                        leaf_pane!(),
                        PaneBranchTemplate {
                            split_direction: launch_config::SplitDirection::Vertical,
                            panes: vec![leaf_pane!(), leaf_pane!()],
                        },
                    ],
                },
                PaneBranchTemplate {
                    split_direction: launch_config::SplitDirection::Horizontal,
                    panes: vec![leaf_pane!(), leaf_pane!(), leaf_pane!()],
                },
            ],
        };

        let window_size = Vector2F::new(window_width, window_height);
        let pane_group = mock_pane_group(
            &mut app,
            MockOptions {
                layout: PanesLayout::Template(template),
                window_bounds: WindowBounds::ExactPosition(RectF::new(
                    Vector2F::zero(),
                    window_size,
                )),
            },
        );

        // Assert that the window created by the call to `mock_pane_group`
        // has the expected bounds.
        let window_id = app.read(|ctx| pane_group.window_id(ctx));
        app.update(|ctx| {
            assert_eq!(
                Some(window_size),
                ctx.window_bounds(&window_id).map(|rect| rect.size())
            );
        });

        let pane_group_width = window_width - 2.0 * workspace::WORKSPACE_PADDING;
        let pane_group_height =
            window_height - workspace::TOTAL_TAB_BAR_HEIGHT - 2.0 * workspace::WORKSPACE_PADDING;

        pane_group.read(&app, |pane_group, ctx| {
            // Make assertions about the expected widths of the various
            // panes.
            assert_eq!(
                pane_group
                    .terminal_view_at_pane_index(0, ctx)
                    .unwrap()
                    .as_ref(ctx)
                    .size_info()
                    .pane_width_px()
                    .as_f32(),
                pane_group_width,
                "Pane with index 0 had unexpected width!"
            );
            let half_width = (pane_group_width - tree::get_divider_thickness()) / 2.;
            for i in 1..=3 {
                assert_eq!(
                    pane_group
                        .terminal_view_at_pane_index(i, ctx)
                        .unwrap()
                        .as_ref(ctx)
                        .size_info()
                        .pane_width_px()
                        .as_f32(),
                    half_width,
                    "Pane with index {i} had unexpected width!"
                );
            }
            let one_third_width = (pane_group_width - (2. * tree::get_divider_thickness())) / 3.;
            for i in 4..=6 {
                assert_eq!(
                    pane_group
                        .terminal_view_at_pane_index(i, ctx)
                        .unwrap()
                        .as_ref(ctx)
                        .size_info()
                        .pane_width_px()
                        .as_f32(),
                    one_third_width,
                    "Pane with index {i} had unexpected width!"
                );
            }

            // Make assertions about the expected heights of the various
            // panes.
            let one_third_height = (pane_group_height - (2. * tree::get_divider_thickness())) / 3.;
            for i in (0..=1).chain(4..=6) {
                assert_eq!(
                    pane_group
                        .terminal_view_at_pane_index(i, ctx)
                        .unwrap()
                        .as_ref(ctx)
                        .size_info()
                        .pane_height_px()
                        .as_f32(),
                    one_third_height,
                    "Pane with index {i} had unexpected height!"
                );
            }
            let one_sixth_height = (pane_group_height - (5. * tree::get_divider_thickness())) / 6.;
            for i in 2..=3 {
                assert_eq!(
                    pane_group
                        .terminal_view_at_pane_index(i, ctx)
                        .unwrap()
                        .as_ref(ctx)
                        .size_info()
                        .pane_height_px()
                        .as_f32(),
                    one_sixth_height,
                    "Pane with index {i} had unexpected height!"
                );
            }
        });
    });
}

#[test]
fn test_is_terminal_pane_being_shared() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let pane_group = mock_pane_group(&mut app, Default::default());
        pane_group.update(&mut app, |panes, ctx| {
            assert!(!panes.is_terminal_pane_being_shared(ctx));

            // Add another pane; the pane group should still be "unshared".
            panes.add_terminal_pane(Direction::Left, None, ctx);
            assert!(!panes.is_terminal_pane_being_shared(ctx));

            // Make one of the terminal panes shared. There is now at least one terminal pane being shared.
            panes
                .terminal_session_by_pane_index(0)
                .expect("terminal pane exists")
                .terminal_manager(ctx)
                .as_ref(ctx)
                .model()
                .lock()
                .set_shared_session_status(SharedSessionStatus::ActiveSharer);
            assert!(panes.is_terminal_pane_being_shared(ctx));
        });
    });
}

#[test]
fn test_number_of_shared_panes() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            // We have two terminal sessions. Neither is shared
            let first_pane_id = get_newly_created_pane_id(panes, &[]);
            panes.add_terminal_pane(Direction::Up, None, ctx);
            assert_eq!(panes.number_of_shared_sessions(ctx), 0);

            // Make one pane shared
            panes
                .terminal_manager(0, ctx)
                .unwrap()
                .as_ref(ctx)
                .model()
                .lock()
                .set_shared_session_status(SharedSessionStatus::ActiveSharer);
            assert_eq!(panes.number_of_shared_sessions(ctx), 1);

            // Make both panes shared
            panes
                .terminal_manager(1, ctx)
                .unwrap()
                .as_ref(ctx)
                .model()
                .lock()
                .set_shared_session_status(SharedSessionStatus::ActiveSharer);
            assert_eq!(panes.number_of_shared_sessions(ctx), 2);

            // Close a pane
            panes.close_pane(first_pane_id, ctx);
            assert_eq!(panes.number_of_shared_sessions(ctx), 1);
        });
    });
}

#[test]
fn test_start_shared_session_from_modal() {
    let _guard = FeatureFlag::CreatingSharedSessions.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |pane_group, ctx| {
            let terminal_pane = pane_group.terminal_session_by_pane_index(0).unwrap();
            let terminal_pane_id = terminal_pane.terminal_pane_id();
            let terminal_model = terminal_pane.terminal_manager(ctx).as_ref(ctx).model();

            assert!(matches!(
                terminal_model.lock().shared_session_status(),
                SharedSessionStatus::NotShared
            ));

            pane_group.open_share_session_modal(
                terminal_pane_id,
                SharedSessionActionSource::PaneHeader,
                ctx,
            );
            assert!(pane_group.terminal_with_open_share_session_modal.is_some());
            assert_eq!(
                pane_group
                    .share_session_modal
                    .as_ref(ctx)
                    .terminal_pane_id(),
                Some(terminal_pane_id)
            );

            pane_group.handle_share_session_modal_event(
                &ShareSessionModalEvent::StartSharing {
                    terminal_pane_id,
                    scrollback_type: SharedSessionScrollbackType::None,
                    source: SharedSessionActionSource::PaneHeader,
                },
                ctx,
            );
            assert!(pane_group.terminal_with_open_share_session_modal.is_none());
            assert!(matches!(
                terminal_model.lock().shared_session_status(),
                SharedSessionStatus::SharePending
            ));
        });

        // Wait for one tick of the event loop for the share to be started.
        pane_group.read(&app, |pane_group, ctx| {
            let terminal_view = pane_group
                .terminal_view_at_pane_index(0, ctx)
                .unwrap()
                .to_owned();
            let model = terminal_view.as_ref(ctx).model.lock();
            assert!(matches!(
                model.shared_session_status(),
                SharedSessionStatus::ActiveSharer
            ));

            let manager = shared_session::manager::Manager::as_ref(ctx);
            let shared_views = manager.shared_views(ctx).collect_vec();
            assert_eq!(shared_views.len(), 1);
            assert_eq!(shared_views[0].id(), terminal_view.id());

            let terminal_pane = pane_group.terminal_session_by_pane_index(0).unwrap();
            assert!(
                terminal_pane
                    .pane_view()
                    .as_ref(ctx)
                    .header()
                    .as_ref(ctx)
                    .has_shareable_object(ctx)
            );
        });
    });
}

/// TODO: look into moving this test somewhere more suitable.
/// Currently, the pane group is responsible for creating and owning
/// the terminal manager, which in turn owns the Network model for the share.
#[test]
fn test_stop_shared_session() {
    let _guard = FeatureFlag::CreatingSharedSessions.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        // Start the shared session.
        pane_group.update(&mut app, |pane_group, ctx| {
            let terminal_pane = pane_group.terminal_session_by_pane_index(0).unwrap();
            let terminal_view = terminal_pane.terminal_view(ctx);
            terminal_view.update(ctx, |terminal_view, ctx| {
                terminal_view.attempt_to_share_session(
                    SharedSessionScrollbackType::None,
                    None,
                    SharedSessionSource::user(None),
                    false,
                    ctx,
                );
            });
        });

        // Wait for one tick of the event loop for the share to be started.
        pane_group.read(&app, |pane_group, ctx| {
            let terminal_model = pane_group
                .terminal_session_by_pane_index(0)
                .unwrap()
                .to_owned()
                .terminal_manager(ctx)
                .as_ref(ctx)
                .model();
            assert!(matches!(
                terminal_model.lock().shared_session_status(),
                SharedSessionStatus::ActiveSharer
            ));
        });

        // Stop the shared session.
        pane_group.update(&mut app, |pane_group, ctx| {
            let terminal_pane = pane_group.terminal_session_by_pane_index(0).unwrap();
            let terminal_view = terminal_pane.terminal_view(ctx);
            terminal_view.update(ctx, |terminal_view, ctx| {
                terminal_view.stop_sharing_session(SharedSessionActionSource::PaneHeader, ctx);
            });
        });

        // Ensure the state is correct after stopping.
        pane_group.update(&mut app, |pane_group, ctx| {
            let terminal_pane = pane_group.terminal_session_by_pane_index(0).unwrap();
            let terminal_manager = terminal_pane
                .terminal_manager(ctx)
                .as_ref(ctx)
                .as_any()
                .downcast_ref::<TerminalManager<TerminalView>>()
                .unwrap();
            let terminal_model = terminal_pane.terminal_manager(ctx).as_ref(ctx).model();

            assert!(terminal_manager.session_sharer().borrow().is_none());
            assert!(matches!(
                terminal_model.lock().shared_session_status(),
                SharedSessionStatus::NotShared
            ));

            let manager = shared_session::manager::Manager::as_ref(ctx);
            let shared_views = manager.shared_views(ctx).collect_vec();
            assert!(shared_views.is_empty());

            assert!(
                !terminal_pane
                    .pane_view()
                    .as_ref(ctx)
                    .header()
                    .as_ref(ctx)
                    .has_shareable_object(ctx)
            );
        });
    });
}

#[test]
fn test_navigation_skips_hidden_closed_panes() {
    let _guard = FeatureFlag::UndoClosedPanes.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            // Add second terminal to the right to create a horizontal pair
            panes.add_terminal_pane(Direction::Right, None, ctx);

            // Add third terminal; place it to the right of current focus
            panes.add_terminal_pane(Direction::Right, None, ctx);

            // Determine ordered visible panes by index 0..2
            let a = panes.pane_id_by_index(0).expect("pane 0 exists");
            let b = panes.pane_id_by_index(1).expect("pane 1 exists");
            let c = panes.pane_id_by_index(2).expect("pane 2 exists");

            // Focus C and confirm prev would be B when all are visible
            panes.focus_pane_by_id(c, ctx);
            assert_eq!(panes.prev_pane_id_navigation(c), Some(b));

            // Close B (it will be hidden for undo and excluded from visible navigation)
            panes.close_pane(b, ctx);

            // Now prev from C should skip B and go to A
            assert_eq!(panes.prev_pane_id_navigation(c), Some(a));

            // And next from A should skip B and go to C
            assert_eq!(panes.next_pane_id(a), Some(c));
        })
    });
}

/// Regression test: closing a host pane on the non-undo `close_pane` branch
/// must clear its entry from `transitively_shared_child_panes`. The undo
/// branch relies on `cleanup_closed_pane` to call
/// `forget_transitively_shared_pane`, but the non-undo branch destroys the
/// pane directly and previously skipped that cleanup, leaking stale entries.
#[test]
fn test_close_pane_clears_transitively_shared_child_entry_on_non_undo_branch() {
    let _undo_closed_panes = FeatureFlag::UndoClosedPanes.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let host_pane_id = get_newly_created_pane_id(panes, &[]);

            // Add a sibling terminal so the host close does not trip the
            // `pane_count() == 1` early return in `close_pane`'s non-undo
            // branch.
            panes.add_terminal_pane(Direction::Right, None, ctx);

            // Cascade an off-tree transitively-shared child onto the host
            // pane id; this populates `transitively_shared_child_panes`.
            let child_pane_id = panes.insert_terminal_pane_hidden_for_child_agent(
                host_pane_id,
                HashMap::new(),
                IsSharedSessionCreator::Yes {
                    source: SharedSessionSource::user(Some("host-task".to_string())),
                },
                ctx,
            );

            assert!(
                panes
                    .transitively_shared_child_panes
                    .get(&host_pane_id)
                    .is_some_and(|children| children.contains(&child_pane_id.into())),
                "setup precondition: host should track its transitively-shared child"
            );

            // Close the host via the non-undo branch.
            panes.close_pane(host_pane_id, ctx);

            assert!(
                !panes
                    .transitively_shared_child_panes
                    .contains_key(&host_pane_id),
                "host entry must be cleared after close_pane on the non-undo branch"
            );
        })
    });
}

// Ensures that we always show the pane header for terminal panes, regardless of split state.
#[test]
fn test_terminal_pane_headers() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        // There should be a single terminal pane to start and the pane header should not be shown.
        pane_group.read(&app, |pane_group, ctx| {
            assert_eq!(pane_group.pane_contents.len(), 1);

            let terminal_panes = pane_group.panes_of::<TerminalPane>().collect_vec();
            assert_eq!(terminal_panes.len(), 1);

            let pane_view = terminal_panes[0].pane_view();
            let header_visible = pane_view
                .as_ref(ctx)
                .header()
                .as_ref(ctx)
                .is_visible_in_pane_group();
            assert!(header_visible);
        });

        // Create a terminal split pane.
        pane_group.update(&mut app, |pane_group, ctx| {
            pane_group.add_terminal_pane(Direction::Left, None, ctx);
        });

        // There should be two terminal panes and they should both have the pane header.
        pane_group.read(&app, |pane_group, ctx| {
            assert_eq!(pane_group.pane_contents.len(), 2);

            let terminal_panes = pane_group.panes_of::<TerminalPane>().collect_vec();
            assert_eq!(terminal_panes.len(), 2);

            for terminal_pane in terminal_panes {
                let pane_view = terminal_pane.pane_view();
                assert!(
                    pane_view
                        .as_ref(ctx)
                        .header()
                        .as_ref(ctx)
                        .is_visible_in_pane_group()
                );
            }
        });

        // Close one of the panes; the remaining pane should still have a header.
        pane_group.update(&mut app, |pane_group, ctx| {
            pane_group.close_pane(pane_group.focused_pane_id(ctx), ctx);
        });

        pane_group.read(&app, |pane_group, ctx| {
            assert_eq!(pane_group.pane_contents.len(), 1);

            let terminal_panes = pane_group.panes_of::<TerminalPane>().collect_vec();
            assert_eq!(terminal_panes.len(), 1);

            let pane_view = terminal_panes[0].pane_view();
            assert!(
                pane_view
                    .as_ref(ctx)
                    .header()
                    .as_ref(ctx)
                    .is_visible_in_pane_group()
            );
        });

        // Create a non-terminal split pane. Terminal pane header remains visible.
        pane_group.update(&mut app, |pane_group, ctx| {
            pane_group.add_pane_with_direction(
                Direction::Left,
                NotebookPane::new(new_notebook(ctx), ctx),
                true, /* focus_new_pane */
                ctx,
            );
        });

        pane_group.read(&app, |pane_group, ctx| {
            assert_eq!(pane_group.pane_contents.len(), 2);

            let terminal_panes = pane_group.panes_of::<TerminalPane>().collect_vec();
            assert_eq!(terminal_panes.len(), 1);

            let pane_view = terminal_panes[0].pane_view();
            assert!(
                pane_view
                    .as_ref(ctx)
                    .header()
                    .as_ref(ctx)
                    .is_visible_in_pane_group()
            );
        });
    });
}

/// Tests that focusing two different panes in quick succession does not cause
/// an infinite loop of focus changes, as outlined in this PR's description:
/// https://github.com/warpdotdev/warp-internal/pull/8990
#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
fn test_pane_focus_does_not_have_an_infinite_event_loop() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Create a pane group with two terminal panes that will fight for
        // focus.
        let mock_options = MockOptions {
            layout: PanesLayout::Template(PaneTemplateType::PaneBranchTemplate {
                split_direction: crate::launch_configs::launch_config::SplitDirection::Horizontal,
                panes: vec![
                    PaneTemplateType::PaneTemplate {
                        is_focused: Some(true),
                        cwd: "/".into(),
                        commands: vec![],
                        pane_mode: PaneMode::Terminal,
                        shell: None,
                    },
                    PaneTemplateType::PaneTemplate {
                        is_focused: None,
                        cwd: "/".into(),
                        commands: vec![],
                        pane_mode: PaneMode::Terminal,
                        shell: None,
                    },
                ],
            }),
            ..Default::default()
        };
        let pane_group = mock_pane_group(&mut app, mock_options);

        // The cycle requires that we are constantly trying to focus the input.
        // An active and long-running block causes focus to move to the
        // terminal instead of the input, so we need to wait until we've
        // finished bootstrapping to ensure no such block will exist.
        assert_eventually!(
            2000 => {
                let mut all_terminals_bootstrapped = true;
                pane_group.update(&mut app, |pane_group, ctx| {
                    pane_group.for_all_terminal_panes(|terminal_view, _ctx| {
                        let model = terminal_view.model.lock();
                        let active_block = model.block_list().active_block();
                        if active_block.bootstrap_stage() != crate::terminal::model::bootstrap::BootstrapStage::PostBootstrapPrecmd ||
                            active_block.is_active_and_long_running() {
                            all_terminals_bootstrapped = false;
                        }
                    }, ctx);
                });
                all_terminals_bootstrapped
            },
            "timed out after ~10s waiting for terminals to finish bootstrapping"
        );

        pane_group.update(&mut app, |pane_group, ctx| {
            // Switch panes twice in quick succession.  We want to make
            // sure the test terminates and doesn't get into an infinite
            // loop.
            pane_group.navigate_next_pane(ctx);
            pane_group.navigate_next_pane(ctx);
        });
    });
}

/// A view to help us react to focus changes and know that they were processed
/// synchronously, not asynchronously (via an Effect::Event).
struct FocusDetectionView {
    pane_group: ViewHandle<PaneGroup>,
    new_focused_pane_id: Option<PaneId>,
}

impl FocusDetectionView {
    fn new(pane_group: ViewHandle<PaneGroup>, ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_view(&pane_group, |me, pane_group, event, ctx| {
            let Event::OpenPromptEditor = event else {
                return;
            };
            // This event is enqueued by us after the `Focus` effect, and so
            // by the time we receive it, application focus will have been
            // moved to the second pane, and (crucially) the pane group should
            // have updated its internal state accordingly (which is what we're
            // asserting here).

            let new_focused_pane_id = me
                .new_focused_pane_id
                .expect("should have set this already");
            pane_group.read(ctx, |pane_group, ctx| {
                assert_eq!(pane_group.focused_pane_id(ctx), new_focused_pane_id);
                assert_eq!(
                    pane_group.active_session_id(ctx),
                    new_focused_pane_id.as_terminal_pane_id()
                );
            });
        });
        Self {
            pane_group,
            new_focused_pane_id: None,
        }
    }
}

impl Entity for FocusDetectionView {
    type Event = ();
}

impl View for FocusDetectionView {
    fn ui_name() -> &'static str {
        "FocusDetectionView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.pane_group).finish()
    }
}

impl TypedActionView for FocusDetectionView {
    type Action = ();
}

/// This test ensures that a change in application focus causes the pane group
/// focused pane to update synchronously, without needing to wait for effect
/// flushing to occur.
///
/// The goal is to avoid situations where a delayed response to application
/// focus changes leads to an infinite loop of focusing and re-focusing two
/// different panes.
#[test]
fn test_focused_pane_is_synchronized_with_application_focus() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Create a pane group with two terminal panes, so that we can move
        // focus and observe the effects.
        let panes_layout = PanesLayout::Template(PaneTemplateType::PaneBranchTemplate {
            split_direction: crate::launch_configs::launch_config::SplitDirection::Horizontal,
            panes: vec![
                PaneTemplateType::PaneTemplate {
                    is_focused: Some(true),
                    cwd: "/".into(),
                    commands: vec![],
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                },
                PaneTemplateType::PaneTemplate {
                    is_focused: None,
                    cwd: "/".into(),
                    commands: vec![],
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                },
            ],
        });

        let tips_model = app.add_model(|_| TipsCompleted::default());
        let (_, root_view) =
            app.add_window_with_bounds(WindowStyle::NotStealFocus, WindowBounds::Default, |ctx| {
                let user_default_shell_changed_banner_dismissal_model_handle =
                    ctx.add_model(|_| BannerState::default());
                let block_lists = Arc::new(HashMap::new());
                let pane_group = ctx.add_typed_action_view(|ctx| {
                    PaneGroup::new_with_panes_layout(
                        tips_model,
                        user_default_shell_changed_banner_dismissal_model_handle,
                        ServerApiProvider::as_ref(ctx).get(),
                        panes_layout,
                        block_lists,
                        AgentSessionRestore::default(),
                        None,
                        ctx,
                    )
                });

                FocusDetectionView::new(pane_group, ctx)
            });
        let pane_group = root_view.read(&app, |root_view, _ctx| root_view.pane_group.clone());

        let (focused_pane_id, active_session_id) = pane_group.read(&app, |pane_group, ctx| {
            (
                pane_group.focused_pane_id(ctx),
                pane_group.active_session_id(ctx),
            )
        });

        let second_pane_id = pane_group.read(&app, |pane_group, _ctx| {
            pane_group
                .pane_ids()
                .find(|pane_id| *pane_id != focused_pane_id)
                .expect("should have more than one pane")
        });

        // Verify that the "second" pane is not focused or active.
        assert_ne!(focused_pane_id, second_pane_id);
        assert_ne!(active_session_id, second_pane_id.as_terminal_pane_id());

        root_view.update(&mut app, |root_view, _ctx| {
            root_view.new_focused_pane_id = Some(second_pane_id);
        });

        pane_group.update(&mut app, |pane_group, ctx| {
            // First, request a change of application focus to the second
            // pane's terminal view.
            pane_group
                .terminal_view_from_pane_id(second_pane_id, ctx)
                .expect("second pane is a terminal pane")
                .update(ctx, |_terminal_view, ctx| {
                    ctx.focus_self();
                });

            // Second, emit an event on the pane group to trigger assertion
            // logic in the FocusDetectionView.  This event effect is enqueued after
            // the focus effect but before the focus effect is processed, meaning
            // it will observe any changes that occurred synchronously as part
            // of the focus effect but will _not_ observe any changes that result
            // from events dispatched during focus handling.
            //
            // We use `OpenPromptEditor` because we can be confident that
            // nothing else above may have emitted this event.
            //
            // IMPORTANT: This MUST be emitted in the same pane group update
            // during which we focus the terminal view, to ensure that the
            // effect queue doesn't get processed or further modified before we
            // enqueue this event on the effect queue.
            ctx.emit(Event::OpenPromptEditor);
        });
    });
}

/// APP-5243: closing a file pane only hides it while undo-close is available, and the same view is
/// reattached without reopening its file. Releasing the file on close would therefore leave a
/// restored pane rendering content that can never update again. The file is released only once the
/// pane is permanently discarded.
#[cfg(feature = "local_fs")]
#[test]
fn test_undo_close_keeps_a_file_pane_watching_its_file() {
    use warp_files::FileModel;

    let _undo_closed_panes = FeatureFlag::UndoClosedPanes.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.add_singleton_model(FileModel::new);
        let pane_group = mock_pane_group(&mut app, Default::default());

        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("notes.md");
        std::fs::write(&path, "# before").expect("write file");

        pane_group.update(&mut app, |panes, ctx| {
            let pane = FilePane::new(
                Some(LocalOrRemotePath::Local(path.clone())),
                None,
                None,
                ctx,
            );
            panes.add_pane_with_direction(Direction::Right, pane, true, ctx);
        });

        let (file_pane_id, file_view) = pane_group.read(&app, |panes, ctx| {
            panes
                .file_notebook_panes(ctx)
                .next()
                .expect("the file pane should exist")
        });

        // Let the read settle so the pane is fully loaded and watching.
        let loaded = file_view.update(&mut app, |view, ctx| {
            let file_id = view.file_id_for_test().expect("the file should be open");
            let future_handle = FileModel::as_ref(ctx)
                .get_future_handle(file_id)
                .expect("Loading future should be present");
            ctx.await_spawned_future(future_handle.future_id())
        });
        loaded.await;

        // Close the way the pane header's close button does, which is the path that reaches
        // `BackingView::close` before the pane group hides the pane.
        file_view.update(&mut app, BackingView::close);
        pane_group.update(&mut app, |panes, ctx| {
            assert!(
                panes.is_pane_hidden_for_close(file_pane_id),
                "closing should hide the pane for undo rather than discard it"
            );
            assert!(
                panes.restore_closed_pane(file_pane_id, ctx),
                "the closed pane should be restorable"
            );
        });

        app.read(|ctx| {
            let file_id = file_view
                .as_ref(ctx)
                .file_id_for_test()
                .expect("a restored pane should still hold its file open");
            assert!(
                FileModel::as_ref(ctx).file_path(file_id).is_some(),
                "a restored pane should still be tracked by the file model"
            );
        });

        // Permanently discarding the pane does release it.
        pane_group.update(&mut app, |panes, ctx| {
            panes.close_pane(file_pane_id, ctx);
            panes.cleanup_closed_pane(file_pane_id, ctx);
        });

        app.read(|ctx| {
            assert!(
                file_view.as_ref(ctx).file_id_for_test().is_none(),
                "a permanently discarded pane should release its file"
            );
        });
    });
}

// A resume can only be offered if the recorded map and the restored pane agree on the key. The
// map is keyed by pane uuid, so the pane the snapshot rebuilds has to report that same uuid.
#[test]
fn restored_terminal_pane_reports_the_uuid_its_recorded_session_is_keyed_by() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let pane_uuid = vec![7, 7, 7];
        let recorded = crate::app_state::RecordedAgentSession {
            agent: crate::terminal::CLIAgent::Claude,
            session_id: "session-1".to_owned(),
            flags: vec![crate::terminal::cli_agent_resume::RecordedFlag {
                name: "--model".to_owned(),
                value: Some("opus".to_owned()),
            }],
            directory: PathBuf::from("/tmp/project"),
            observed_at: chrono::NaiveDate::from_ymd_opt(2026, 8, 11)
                .expect("date should be valid")
                .and_hms_opt(9, 30, 0)
                .expect("time should be valid"),
        };
        let agent_restore = AgentSessionRestore {
            sessions: Arc::new(HashMap::from([(
                PaneUuid(pane_uuid.clone()),
                recorded.clone(),
            )])),
            claimed_panes: Arc::new(HashSet::from([PaneUuid(pane_uuid.clone())])),
            is_startup_restore: true,
        };

        let layout = PanesLayout::Snapshot(Box::new(PaneNodeSnapshot::Leaf(LeafSnapshot {
            is_focused: true,
            custom_vertical_tabs_title: None,
            contents: LeafContents::Terminal(TerminalPaneSnapshot {
                uuid: pane_uuid,
                cwd: None,
                shell_launch_data: None,
                is_active: true,
                is_read_only: false,
                input_config: None,
                llm_model_override: None,
                active_profile_id: None,
                conversation_ids_to_restore: vec![],
                active_conversation_id: None,
            }),
        })));

        let tips_model = app.add_model(|_| TipsCompleted::default());
        let restore_for_group = agent_restore.clone();
        let (_, pane_group) = app.add_window_with_bounds(
            WindowStyle::NotStealFocus,
            WindowBounds::ExactPosition(RectF::new(Vector2F::zero(), Vector2F::new(1024., 768.))),
            |ctx| {
                let banner_model_handle = ctx.add_model(|_| BannerState::default());
                PaneGroup::new_with_panes_layout(
                    tips_model,
                    banner_model_handle,
                    ServerApiProvider::as_ref(ctx).get(),
                    layout,
                    Arc::new(HashMap::new()),
                    restore_for_group,
                    None,
                    ctx,
                )
            },
        );

        let reported_uuid = pane_group.read(&app, |panes, _ctx| {
            panes
                .panes_of::<TerminalPane>()
                .map(|pane| pane.session_uuid())
                .next()
                .expect("the snapshot should have restored a terminal pane")
        });

        assert_eq!(
            agent_restore.recorded_on_startup(&PaneUuid(reported_uuid)),
            Some(&recorded)
        );
    });
}

/// A pane group whose panes report their persistence writes to the returned receiver, so a test
/// can read exactly what a pane asked the writer thread to store.
fn pane_group_reporting_model_events(
    app: &mut App,
) -> (ViewHandle<PaneGroup>, Receiver<ModelEvent>) {
    let (sender, receiver) = std::sync::mpsc::sync_channel(64);
    let tips_model = app.add_model(|_| TipsCompleted::default());
    let (_, pane_group) = app.add_window_with_bounds(
        WindowStyle::NotStealFocus,
        WindowBounds::ExactPosition(RectF::new(Vector2F::zero(), Vector2F::new(1024., 768.))),
        |ctx| {
            let banner_model_handle = ctx.add_model(|_| BannerState::default());
            PaneGroup::new_with_panes_layout(
                tips_model,
                banner_model_handle,
                ServerApiProvider::as_ref(ctx).get(),
                Default::default(),
                Arc::new(HashMap::new()),
                AgentSessionRestore::default(),
                Some(sender),
                ctx,
            )
        },
    );
    (pane_group, receiver)
}

/// Starts a CLI agent session for `terminal_view_id`, as detection does once a recognized agent
/// has been the pane's foreground command for long enough.
fn start_cli_agent_session(
    terminal_view_id: EntityId,
    agent: crate::terminal::CLIAgent,
    ctx: &mut ViewContext<PaneGroup>,
) {
    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
        sessions.set_session(
            terminal_view_id,
            CLIAgentSession {
                agent,
                status: CLIAgentSessionStatus::InProgress,
                session_context: CLIAgentSessionContext::default(),
                input_state: CLIAgentInputState::Closed,
                should_auto_toggle_input: false,
                listener: None,
                plugin_version: None,
                remote_host: None,
                draft_text: None,
                custom_command_prefix: None,
                received_rich_notification: false,
            },
            ctx,
        );
    });
}

/// Delivers the plugin event in which the agent reports `session_id`, the same shape the OSC 777
/// listener parses out of the PTY.
fn report_agent_session_id(
    terminal_view_id: EntityId,
    session_id: &str,
    ctx: &mut ViewContext<PaneGroup>,
) {
    report_agent_event(terminal_view_id, "session_start", session_id, ctx);
}

/// Delivers one of the agent's own lifecycle events. `tool_complete` is the one that fires once
/// per tool call, which is what makes an agent task a burst rather than a handful of events.
fn report_agent_event(
    terminal_view_id: EntityId,
    event: &str,
    session_id: &str,
    ctx: &mut ViewContext<PaneGroup>,
) {
    let body =
        format!(r#"{{"v":1,"agent":"claude","event":"{event}","session_id":"{session_id}"}}"#);
    let event = parse_event(Some("warp://cli-agent"), &body).expect("the test event should parse");
    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
        sessions.update_from_event(terminal_view_id, &event, ctx);
    });
}

/// The capture writes the panes sent, oldest first. A `None` session is a pane reporting that it
/// has no agent left to resume.
fn captured_agent_session_writes(
    events: &Receiver<ModelEvent>,
) -> Vec<(Vec<u8>, Option<RecordedAgentSession>)> {
    events
        .try_iter()
        .filter_map(|event| match event {
            ModelEvent::SetAgentSession { pane_id, session } => Some((pane_id, session)),
            _ => None,
        })
        .collect()
}

/// The identifiers the panes recorded, oldest first.
fn captured_agent_sessions(events: &Receiver<ModelEvent>) -> Vec<(Vec<u8>, String)> {
    captured_agent_session_writes(events)
        .into_iter()
        .filter_map(|(pane_id, session)| Some((pane_id, session?.session_id)))
        .collect()
}

/// Completes a block in the pane's terminal, which is how the agent process exiting (a `User`
/// block) and the agent being suspended (a `Background` block) reach the sessions model.
fn complete_block_in_pane(
    terminal_view: &ViewHandle<TerminalView>,
    block_type: BlockType,
    ctx: &mut ViewContext<PaneGroup>,
) {
    let dispatcher = terminal_view
        .as_ref(ctx)
        .model_event_dispatcher()
        .to_owned();
    dispatcher.update(ctx, |_, ctx| {
        ctx.emit(TerminalModelEvent::BlockCompleted(BlockCompletedEvent {
            block_type,
            num_secrets_obfuscated: 0,
            block_index: BlockIndex::zero(),
            block_id: BlockId::new(),
            session_id: None,
            restored_block_was_local: None,
        }));
    });
}

/// The block a finished agent command leaves behind.
fn completed_user_block(command: &str) -> BlockType {
    BlockType::User(UserBlockCompleted {
        index: BlockIndex::zero(),
        serialized_block: Arc::new(SerializedBlock::new_for_test(
            command.as_bytes().to_vec(),
            vec![],
        )),
        command: command.to_owned(),
        command_with_obfuscated_secrets: command.to_owned(),
        output_truncated: String::new(),
        output_truncated_with_obfuscated_secrets: String::new(),
        was_part_of_agent_interaction: false,
        was_warp_authored: false,
        started_at: None,
        num_output_lines: 0,
        num_output_lines_truncated: 0,
    })
}

/// The uuid of the group's only terminal pane, which is the key its recorded state is stored
/// under.
fn only_terminal_pane_uuid(pane_group: &ViewHandle<PaneGroup>, app: &App) -> Vec<u8> {
    pane_group.read(app, |panes, _ctx| {
        panes
            .panes_of::<TerminalPane>()
            .map(|pane| pane.session_uuid())
            .next()
            .expect("the group should hold one terminal pane")
    })
}

/// The id of the group's first terminal pane, for tests that add a second one afterwards.
fn first_terminal_pane_id(pane_group: &ViewHandle<PaneGroup>, app: &App) -> PaneId {
    pane_group.read(app, |panes, _ctx| {
        panes
            .terminal_pane_ids()
            .next()
            .expect("the group should hold a terminal pane")
    })
}

/// The uuid recorded state is keyed to for the terminal pane with `pane_id`.
fn terminal_pane_uuid(pane_group: &ViewHandle<PaneGroup>, pane_id: PaneId, app: &App) -> Vec<u8> {
    pane_group.read(app, |panes, _ctx| {
        panes
            .terminal_session_by_id(pane_id)
            .expect("the group should hold a terminal pane with that id")
            .session_uuid()
    })
}

// AE1/R2: a pane whose agent reports a second identifier has to persist the second one, and the
// writes have to reach the writer in the order they were observed — an older identifier landing
// after a newer one would resume the wrong conversation.
#[test]
fn pane_records_each_identifier_its_agent_reports_in_order() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (pane_group, model_events) = pane_group_reporting_model_events(&mut app);
        let pane_uuid = only_terminal_pane_uuid(&pane_group, &app);

        let terminal_view_id = pane_group.update(&mut app, |panes, ctx| {
            let terminal_view_id = panes
                .active_session_view(ctx)
                .expect("the group should have an active terminal view")
                .id();
            start_cli_agent_session(terminal_view_id, crate::terminal::CLIAgent::Claude, ctx);
            terminal_view_id
        });

        pane_group.update(&mut app, |_, ctx| {
            report_agent_session_id(terminal_view_id, "first-identifier", ctx);
        });
        pane_group.update(&mut app, |_, ctx| {
            report_agent_session_id(terminal_view_id, "second-identifier", ctx);
        });

        assert_eq!(
            captured_agent_sessions(&model_events),
            vec![
                (pane_uuid.clone(), "first-identifier".to_owned()),
                (pane_uuid, "second-identifier".to_owned()),
            ],
            "both identifiers must reach the writer in the order the agent reported them"
        );
    });
}

/// Starts an agent in the group's terminal pane and has it report `session_id`, leaving one
/// recorded write behind. Returns the pane's terminal view.
fn record_agent_session_in_pane(
    pane_group: &ViewHandle<PaneGroup>,
    session_id: &str,
    app: &mut App,
) -> ViewHandle<TerminalView> {
    let terminal_view = pane_group.update(app, |panes, ctx| {
        let terminal_view = panes
            .active_session_view(ctx)
            .expect("the group should have an active terminal view");
        start_cli_agent_session(terminal_view.id(), crate::terminal::CLIAgent::Claude, ctx);
        terminal_view
    });
    let terminal_view_id = terminal_view.id();
    pane_group.update(app, |_, ctx| {
        report_agent_session_id(terminal_view_id, session_id, ctx);
    });
    terminal_view
}

// AE2/R3: the agent process exiting completes the pane's user block, which ends the session while
// the pane stays attached. That pane has nothing left to resume, and saying so is the only way
// the next launch does not offer a dead session.
#[test]
fn pane_whose_agent_exited_records_that_it_has_nothing_to_resume() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (pane_group, model_events) = pane_group_reporting_model_events(&mut app);
        let pane_uuid = only_terminal_pane_uuid(&pane_group, &app);
        let terminal_view = record_agent_session_in_pane(&pane_group, "conversation-a", &mut app);
        let _ = captured_agent_session_writes(&model_events);

        pane_group.update(&mut app, |_, ctx| {
            complete_block_in_pane(&terminal_view, completed_user_block("claude"), ctx);
        });

        assert_eq!(
            captured_agent_session_writes(&model_events),
            vec![(pane_uuid, None)],
            "an agent that ended in a live pane must leave that pane recording no session"
        );
    });
}

// AE16/R21: suspending the agent completes a background block, which leaves the session — and the
// process — alive. The pane must keep what it recorded; the agent is still there to resume.
#[test]
fn pane_whose_agent_is_suspended_keeps_its_recorded_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (pane_group, model_events) = pane_group_reporting_model_events(&mut app);
        let terminal_view = record_agent_session_in_pane(&pane_group, "conversation-a", &mut app);
        let recorded = captured_agent_sessions(&model_events);
        assert_eq!(
            recorded.len(),
            1,
            "precondition: the pane recorded a session"
        );

        pane_group.update(&mut app, |_, ctx| {
            complete_block_in_pane(
                &terminal_view,
                BlockType::Background(Arc::new(SerializedBlock::new_for_test(
                    b"claude".to_vec(),
                    vec![],
                ))),
                ctx,
            );
        });

        assert_eq!(
            captured_agent_session_writes(&model_events),
            vec![],
            "a suspended agent must not make the pane clear what it recorded"
        );
    });
}

// AE15/R20: a pane hidden for a close the user can undo, and a pane torn down at app teardown,
// both end their CLI agent session on the way out. Neither says anything about the agent, which
// is still running — clearing there would erase exactly the state a restart needs.
#[test]
fn pane_detached_for_close_or_teardown_keeps_its_recorded_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (pane_group, model_events) = pane_group_reporting_model_events(&mut app);
        record_agent_session_in_pane(&pane_group, "conversation-a", &mut app);
        let recorded = captured_agent_sessions(&model_events);
        assert_eq!(
            recorded.len(),
            1,
            "precondition: the pane recorded a session"
        );

        // Closing the tab hides its panes so an undo can bring them back, and closing the window
        // at teardown detaches every pane down the same path.
        pane_group.update(&mut app, |panes, ctx| panes.detach_panes(ctx));
        assert_eq!(
            captured_agent_session_writes(&model_events),
            vec![],
            "a pane hidden for close must keep its recorded state so an undo (and the next \
             launch) still finds the agent it was running"
        );
    });
}

// AE15/R20: the undo that the hide-for-close exists for. The pane comes back under the uuid its
// recorded state is keyed to, and nothing on the way out or the way back said that state was
// stale, so the next launch still resumes it.
#[test]
fn undone_close_leaves_the_pane_still_owning_its_recorded_state() {
    let _undo_closed_panes = FeatureFlag::UndoClosedPanes.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (pane_group, model_events) = pane_group_reporting_model_events(&mut app);
        record_agent_session_in_pane(&pane_group, "conversation-a", &mut app);
        let recorded = captured_agent_sessions(&model_events);
        assert_eq!(
            recorded.len(),
            1,
            "precondition: the pane recorded a session"
        );
        let agent_pane_id = first_terminal_pane_id(&pane_group, &app);
        let pane_uuid = terminal_pane_uuid(&pane_group, agent_pane_id, &app);

        // A second pane, so closing the agent's pane hides it rather than emptying the group.
        pane_group.update(&mut app, |panes, ctx| {
            panes.add_terminal_pane(Direction::Right, None, ctx);
        });
        pane_group.update(&mut app, |panes, ctx| panes.close_pane(agent_pane_id, ctx));
        assert!(
            pane_group.read(&app, |panes, _ctx| panes
                .is_pane_hidden_for_close(agent_pane_id)),
            "precondition: the close hid the pane instead of removing it"
        );

        assert!(
            pane_group.update(&mut app, |panes, ctx| panes
                .restore_closed_pane(agent_pane_id, ctx)),
            "the hidden pane should restore"
        );

        assert_eq!(
            captured_agent_session_writes(&model_events),
            vec![],
            "an undone close must leave the recorded state exactly as the pane left it"
        );
        assert_eq!(
            terminal_pane_uuid(&pane_group, agent_pane_id, &app),
            pane_uuid,
            "the restored pane is the same pane, so it still owns the row keyed to its uuid"
        );
    });
}

// R20/KTD13: once the undo window has passed, the stack discards the closed item and detaches its
// panes as permanently closed. Nothing brings that pane back, so the row keyed to its uuid is
// garbage whatever its agent was doing, and the same hook that drops its blocks drops it too.
#[test]
fn permanently_removed_pane_has_its_recorded_state_cleared() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (pane_group, model_events) = pane_group_reporting_model_events(&mut app);
        let pane_uuid = only_terminal_pane_uuid(&pane_group, &app);
        record_agent_session_in_pane(&pane_group, "conversation-a", &mut app);
        let recorded = captured_agent_sessions(&model_events);
        assert_eq!(
            recorded.len(),
            1,
            "precondition: the pane recorded a session"
        );

        // What the undo stack runs when it discards a closed tab for good.
        pane_group.update(&mut app, |panes, ctx| panes.clean_up_panes(ctx));

        assert_eq!(
            captured_agent_session_writes(&model_events),
            vec![(pane_uuid, None)],
            "a pane that is gone for good must not leave a row behind for a launch to resume"
        );
    });
}

// A move detaches the pane from the group it is leaving, but the pane, its uuid and its running
// agent all survive into the destination. Clearing there would lose the state mid-drag.
#[test]
fn pane_moved_out_of_its_group_keeps_its_recorded_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (pane_group, model_events) = pane_group_reporting_model_events(&mut app);
        record_agent_session_in_pane(&pane_group, "conversation-a", &mut app);
        let recorded = captured_agent_sessions(&model_events);
        assert_eq!(
            recorded.len(),
            1,
            "precondition: the pane recorded a session"
        );
        let agent_pane_id = first_terminal_pane_id(&pane_group, &app);

        pane_group.update(&mut app, |panes, ctx| {
            panes.add_terminal_pane(Direction::Right, None, ctx);
        });
        let moved = pane_group.update(&mut app, |panes, ctx| {
            panes.remove_pane_for_move(&agent_pane_id, ctx)
        });
        assert!(
            moved.is_some(),
            "precondition: the pane was taken for a move"
        );

        assert_eq!(
            captured_agent_session_writes(&model_events),
            vec![],
            "a pane that only moved is still running its agent and must keep what it recorded"
        );
    });
}

// KTD14: the session event fires once per tool call, so an agent task is a burst of observations
// of a row that has not changed. Only what changed is worth a write — the channel these go down
// is bounded and shared with a sender that blocks the main thread when it fills.
#[test]
fn burst_of_tool_call_events_from_one_agent_collapses_to_one_write() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (pane_group, model_events) = pane_group_reporting_model_events(&mut app);
        let terminal_view = record_agent_session_in_pane(&pane_group, "conversation-a", &mut app);
        let terminal_view_id = terminal_view.id();

        for _ in 0..20 {
            pane_group.update(&mut app, |_, ctx| {
                report_agent_event(terminal_view_id, "tool_complete", "conversation-a", ctx);
            });
        }

        assert_eq!(
            captured_agent_sessions(&model_events)
                .into_iter()
                .map(|(_, session_id)| session_id)
                .collect::<Vec<_>>(),
            vec!["conversation-a".to_owned()],
            "an agent task's worth of tool calls must cost one write, not one per call"
        );
    });
}

// Starting a second agent in the same pane ends the first session with the second one already
// registered. The pane is still running an agent, so it has something to resume and must not
// record that it has nothing.
#[test]
fn pane_that_replaced_its_agent_does_not_record_an_absent_session() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (pane_group, model_events) = pane_group_reporting_model_events(&mut app);
        let terminal_view = record_agent_session_in_pane(&pane_group, "conversation-a", &mut app);
        let terminal_view_id = terminal_view.id();
        let _ = captured_agent_session_writes(&model_events);

        pane_group.update(&mut app, |_, ctx| {
            start_cli_agent_session(terminal_view_id, crate::terminal::CLIAgent::Codex, ctx);
        });

        assert_eq!(
            captured_agent_session_writes(&model_events),
            vec![],
            "a pane that swapped one agent for another still has an agent to resume"
        );
    });
}

// The eligibility gate reads a recorded directory that still resolves as the proof that the pane
// ran its agent here. A pane whose session is not local has no directory to report, and giving
// it one would let a session that never ran on this machine be relaunched on it.
#[test]
fn pane_without_a_local_directory_records_none_and_stays_ineligible() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (pane_group, model_events) = pane_group_reporting_model_events(&mut app);
        let pane_uuid = only_terminal_pane_uuid(&pane_group, &app);
        record_agent_session_in_pane(&pane_group, "conversation-a", &mut app);

        let writes = captured_agent_session_writes(&model_events);
        let (_, session) = writes.first().expect("the pane should have recorded state");
        let session = session
            .as_ref()
            .expect("the recording should hold the reported session")
            .clone();
        assert_eq!(
            session.directory,
            PathBuf::new(),
            "a pane that reports no local working directory must record no directory"
        );

        let restore = startup_restore_for_test(
            [(PaneUuid(pane_uuid.clone()), session)],
            [PaneUuid(pane_uuid.clone())],
        );
        assert_eq!(
            resume_eligibility(
                &restore,
                &PaneUuid(pane_uuid.clone()),
                &local_pane_snapshot_for_test(&pane_uuid, Some(Path::new("/tmp"))),
                Some(Path::new("/tmp")),
            ),
            Err(ResumeIneligibility::RecordedDirectoryMissing),
            "a recording without a directory must not pass the gate that treats one as proof the \
             agent ran here"
        );
    });
}

// The capture is part of session restore, so a user who turned session restore off has nothing
// recorded about their agents at all.
#[test]
fn pane_records_nothing_when_session_restore_is_off() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        GeneralSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .restore_session
                .set_value(false, ctx)
                .expect("the setting should be writable in tests");
        });
        let (pane_group, model_events) = pane_group_reporting_model_events(&mut app);
        let terminal_view = record_agent_session_in_pane(&pane_group, "conversation-a", &mut app);

        pane_group.update(&mut app, |_, ctx| {
            complete_block_in_pane(&terminal_view, completed_user_block("claude"), ctx);
        });

        assert_eq!(
            captured_agent_session_writes(&model_events),
            vec![],
            "with session restore off, a pane records neither its agent nor its absence"
        );
    });
}

// R19: the persisted value is a purpose-built struct, and the session context it is derived from
// carries the user's prompts, the agent's replies, its summaries and its tool previews. None of
// that may reach the store, so this pins the recorded field set exhaustively and checks each
// field against a context stuffed with every sensitive value the model can hold.
#[test]
fn recorded_agent_session_carries_no_prompt_response_summary_or_tool_preview() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (pane_group, model_events) = pane_group_reporting_model_events(&mut app);

        let terminal_view_id = pane_group.update(&mut app, |panes, ctx| {
            let terminal_view_id = panes
                .active_session_view(ctx)
                .expect("the group should have an active terminal view")
                .id();
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    terminal_view_id,
                    CLIAgentSession {
                        agent: crate::terminal::CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext {
                            cwd: Some("SENSITIVE-cwd".to_owned()),
                            project: Some("SENSITIVE-project".to_owned()),
                            session_id: Some("conversation-a".to_owned()),
                            tool_name: Some("SENSITIVE-tool-name".to_owned()),
                            tool_input_preview: Some("SENSITIVE-tool-preview".to_owned()),
                            summary: Some("SENSITIVE-summary".to_owned()),
                            query: Some("SENSITIVE-prompt".to_owned()),
                            response: Some("SENSITIVE-response".to_owned()),
                        },
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        plugin_version: None,
                        remote_host: None,
                        draft_text: Some("SENSITIVE-draft".to_owned()),
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });
            terminal_view_id
        });
        pane_group.update(&mut app, |_, ctx| {
            report_agent_session_id(terminal_view_id, "conversation-a", ctx);
        });

        let writes = captured_agent_session_writes(&model_events);
        let (_, session) = writes.first().expect("the pane should have recorded state");
        let session = session
            .as_ref()
            .expect("the recording should hold the reported session");
        // Destructured exhaustively on purpose: a field added to the recorded state has to be
        // looked at here before it can be persisted.
        let RecordedAgentSession {
            agent,
            session_id,
            flags,
            directory,
            observed_at,
        } = session;
        let persisted = format!(
            "{agent:?} {session_id} {flags:?} {} {observed_at}",
            directory.display()
        );
        assert!(
            !persisted.contains("SENSITIVE"),
            "the recorded state must carry nothing of the session context but the identifier, \
             got: {persisted}"
        );
        assert_eq!(session_id, "conversation-a");
    });
}

/// A recording for a pane that was running Claude in `directory` under `session_id`.
fn recorded_session_for_test(session_id: &str, directory: &Path) -> RecordedAgentSession {
    RecordedAgentSession {
        agent: crate::terminal::CLIAgent::Claude,
        session_id: session_id.to_owned(),
        flags: vec![],
        directory: directory.to_path_buf(),
        observed_at: chrono::NaiveDate::from_ymd_opt(2026, 8, 11)
            .expect("date should be valid")
            .and_hms_opt(9, 30, 0)
            .expect("time should be valid"),
    }
}

/// A snapshot of a first-party local terminal pane that came up in `cwd`. A local pane always
/// carries an input config and a cwd; the branches that null either belong to panes the gate has
/// to reject.
fn local_pane_snapshot_for_test(uuid: &[u8], cwd: Option<&Path>) -> TerminalPaneSnapshot {
    TerminalPaneSnapshot {
        uuid: uuid.to_vec(),
        cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
        shell_launch_data: None,
        is_active: true,
        is_read_only: false,
        input_config: Some(InputConfig {
            input_type: crate::ai::blocklist::InputType::Shell,
            is_locked: false,
        }),
        llm_model_override: None,
        active_profile_id: None,
        conversation_ids_to_restore: vec![],
        active_conversation_id: None,
    }
}

fn startup_restore_for_test(
    sessions: impl IntoIterator<Item = (PaneUuid, RecordedAgentSession)>,
    claimed_panes: impl IntoIterator<Item = PaneUuid>,
) -> AgentSessionRestore {
    AgentSessionRestore {
        sessions: Arc::new(sessions.into_iter().collect()),
        claimed_panes: Arc::new(claimed_panes.into_iter().collect()),
        is_startup_restore: true,
    }
}

/// A window snapshot holding `panes` in one tab, so claim resolution has a window layout to
/// reason about without a window existing.
fn window_snapshot_for_test(panes: Vec<TerminalPaneSnapshot>) -> WindowSnapshot {
    WindowSnapshot {
        tabs: vec![crate::app_state::TabSnapshot {
            custom_title: None,
            root: PaneNodeSnapshot::Branch(BranchSnapshot {
                direction: crate::app_state::SplitDirection::Horizontal,
                children: panes
                    .into_iter()
                    .map(|pane| {
                        (
                            crate::app_state::PaneFlex(1.),
                            PaneNodeSnapshot::Leaf(LeafSnapshot {
                                is_focused: false,
                                custom_vertical_tabs_title: None,
                                contents: LeafContents::Terminal(pane),
                            }),
                        )
                    })
                    .collect(),
            }),
            default_directory_color: None,
            selected_color: Default::default(),
            left_panel: None,
            right_panel: None,
            group_id: None,
            pinned: false,
        }],
        active_tab_index: 0,
        team_uid: None,
        bounds: None,
        fullscreen_state: Default::default(),
        quake_mode: false,
        universal_search_width: None,
        warp_ai_width: None,
        voltron_width: None,
        warp_drive_index_width: None,
        left_panel_open: false,
        vertical_tabs_panel_open: false,
        left_panel_width: None,
        right_panel_width: None,
        agent_management_filters: None,
        tab_groups: vec![],
    }
}

// AE8: a pane recorded in a git worktree that was deleted before the restart restores as a plain
// shell. Resuming would run the agent in the fallback directory, which is not where it was.
#[test]
fn resume_is_ineligible_when_the_recorded_directory_no_longer_exists() {
    let worktree = tempfile::tempdir().expect("temp dir");
    let recorded_directory = worktree.path().to_path_buf();
    let pane_uuid = PaneUuid(vec![1]);
    let agent_restore = startup_restore_for_test(
        [(
            pane_uuid.clone(),
            recorded_session_for_test("session-1", &recorded_directory),
        )],
        [pane_uuid.clone()],
    );
    let snapshot = local_pane_snapshot_for_test(&pane_uuid.0, Some(&recorded_directory));
    worktree.close().expect("the worktree should be removable");

    assert_eq!(
        resume_eligibility(&agent_restore, &pane_uuid, &snapshot, None),
        Err(ResumeIneligibility::RecordedDirectoryMissing)
    );
}

// AE12: the recorded directory still resolves, but the pane's shell came up somewhere else, so
// the session belongs to a directory this pane is not in.
#[test]
fn resume_is_ineligible_when_the_pane_restored_into_another_directory() {
    let recorded_directory = tempfile::tempdir().expect("temp dir");
    let restored_directory = tempfile::tempdir().expect("temp dir");
    let pane_uuid = PaneUuid(vec![1]);
    let agent_restore = startup_restore_for_test(
        [(
            pane_uuid.clone(),
            recorded_session_for_test("session-1", recorded_directory.path()),
        )],
        [pane_uuid.clone()],
    );
    let snapshot = local_pane_snapshot_for_test(&pane_uuid.0, Some(restored_directory.path()));

    assert_eq!(
        resume_eligibility(
            &agent_restore,
            &pane_uuid,
            &snapshot,
            Some(restored_directory.path())
        ),
        Err(ResumeIneligibility::RestoredElsewhere)
    );
}

// A pane that came up in the directory its session was recorded in is the only shape that
// resumes, so the gate has to say yes to it.
#[test]
fn resume_is_eligible_when_the_pane_restored_into_its_recorded_directory() {
    let directory = tempfile::tempdir().expect("temp dir");
    let pane_uuid = PaneUuid(vec![1]);
    let recorded = recorded_session_for_test("session-1", directory.path());
    let agent_restore =
        startup_restore_for_test([(pane_uuid.clone(), recorded.clone())], [pane_uuid.clone()]);
    let snapshot = local_pane_snapshot_for_test(&pane_uuid.0, Some(directory.path()));

    assert_eq!(
        resume_eligibility(
            &agent_restore,
            &pane_uuid,
            &snapshot,
            Some(directory.path())
        ),
        Ok(&recorded)
    );
}

// AE9: two panes in different windows recorded one identifier. The claim is resolved over the
// whole store before any window is created — this test creates none — and it goes to the pane in
// the window the user lands in, which restore creates last.
#[test]
fn resume_claims_go_to_the_pane_in_the_window_the_user_lands_in() {
    let directory = tempfile::tempdir().expect("temp dir");
    let background_pane = PaneUuid(vec![1]);
    let landing_pane = PaneUuid(vec![2]);
    let undisputed_pane = PaneUuid(vec![3]);
    let sessions = HashMap::from([
        (
            background_pane.clone(),
            recorded_session_for_test("shared", directory.path()),
        ),
        (
            landing_pane.clone(),
            recorded_session_for_test("shared", directory.path()),
        ),
        (
            undisputed_pane.clone(),
            recorded_session_for_test("its-own", directory.path()),
        ),
    ]);
    let windows = vec![
        window_snapshot_for_test(vec![
            local_pane_snapshot_for_test(&background_pane.0, Some(directory.path())),
            local_pane_snapshot_for_test(&undisputed_pane.0, Some(directory.path())),
        ]),
        window_snapshot_for_test(vec![local_pane_snapshot_for_test(
            &landing_pane.0,
            Some(directory.path()),
        )]),
    ];

    let claims = resolve_agent_session_claims(&windows, Some(1), &sessions);

    assert_eq!(
        claims,
        HashSet::from([landing_pane.clone(), undisputed_pane.clone()]),
        "the disputed identifier goes to the landing window, and an undisputed one is untouched"
    );

    // The landing window is not a position in the list: with the same layout landing elsewhere,
    // the identifier follows the user rather than the restore order.
    let claims = resolve_agent_session_claims(&windows, Some(0), &sessions);

    assert_eq!(claims, HashSet::from([background_pane, undisputed_pane]));
}

// AE9: exactly one pane resumes per identifier, so the pane that lost the claim is ineligible
// even though everything about the pane itself is fine.
#[test]
fn resume_is_ineligible_for_the_pane_that_lost_a_duplicated_identifier() {
    let directory = tempfile::tempdir().expect("temp dir");
    let losing_pane = PaneUuid(vec![1]);
    let winning_pane = PaneUuid(vec![2]);
    let agent_restore = startup_restore_for_test(
        [
            (
                losing_pane.clone(),
                recorded_session_for_test("shared", directory.path()),
            ),
            (
                winning_pane.clone(),
                recorded_session_for_test("shared", directory.path()),
            ),
        ],
        [winning_pane.clone()],
    );

    assert_eq!(
        resume_eligibility(
            &agent_restore,
            &losing_pane,
            &local_pane_snapshot_for_test(&losing_pane.0, Some(directory.path())),
            Some(directory.path())
        ),
        Err(ResumeIneligibility::IdentifierClaimedByAnotherPane)
    );
    assert!(
        resume_eligibility(
            &agent_restore,
            &winning_pane,
            &local_pane_snapshot_for_test(&winning_pane.0, Some(directory.path())),
            Some(directory.path())
        )
        .is_ok(),
        "the pane that won the identifier still resumes"
    );
}

// AE4: a pane running a recognized agent that never reported an identifier has nothing to
// reattach to, and Warp picks a session on no other basis.
#[test]
fn resume_is_ineligible_without_a_recorded_identifier() {
    let directory = tempfile::tempdir().expect("temp dir");
    let pane_uuid = PaneUuid(vec![1]);
    let mut recorded = recorded_session_for_test("", directory.path());
    recorded.session_id = String::new();
    let agent_restore =
        startup_restore_for_test([(pane_uuid.clone(), recorded)], [pane_uuid.clone()]);

    assert_eq!(
        resume_eligibility(
            &agent_restore,
            &pane_uuid,
            &local_pane_snapshot_for_test(&pane_uuid.0, Some(directory.path())),
            Some(directory.path())
        ),
        Err(ResumeIneligibility::NoSessionIdentifier)
    );
}

// An agent with no resume declaration has no invocation that reattaches, so a recording for it
// can only be dropped.
#[test]
fn resume_is_ineligible_for_an_agent_without_a_resume_declaration() {
    let directory = tempfile::tempdir().expect("temp dir");
    let pane_uuid = PaneUuid(vec![1]);
    let mut recorded = recorded_session_for_test("session-1", directory.path());
    recorded.agent = crate::terminal::CLIAgent::Gemini;
    let agent_restore =
        startup_restore_for_test([(pane_uuid.clone(), recorded)], [pane_uuid.clone()]);

    assert_eq!(
        resume_eligibility(
            &agent_restore,
            &pane_uuid,
            &local_pane_snapshot_for_test(&pane_uuid.0, Some(directory.path())),
            Some(directory.path())
        ),
        Err(ResumeIneligibility::AgentNotDeclared)
    );
}

// R16: a pane whose session ran over SSH restores as a local shell, where the recorded
// identifier means nothing. The save path proves locality by writing a cwd only for a local
// session, so a snapshot without one is not a pane to resume in.
#[test]
fn resume_is_ineligible_for_a_pane_whose_session_was_not_local() {
    let directory = tempfile::tempdir().expect("temp dir");
    let pane_uuid = PaneUuid(vec![1]);
    let agent_restore = startup_restore_for_test(
        [(
            pane_uuid.clone(),
            recorded_session_for_test("session-1", directory.path()),
        )],
        [pane_uuid.clone()],
    );

    assert_eq!(
        resume_eligibility(
            &agent_restore,
            &pane_uuid,
            &local_pane_snapshot_for_test(&pane_uuid.0, None),
            None
        ),
        Err(ResumeIneligibility::SessionNotLocal)
    );
}

// A viewer of someone else's shared session never ran the agent locally. Its snapshot is written
// by the viewer branch, which carries no input config.
#[test]
fn resume_is_ineligible_for_a_shared_session_viewer_pane() {
    let directory = tempfile::tempdir().expect("temp dir");
    let pane_uuid = PaneUuid(vec![1]);
    let agent_restore = startup_restore_for_test(
        [(
            pane_uuid.clone(),
            recorded_session_for_test("session-1", directory.path()),
        )],
        [pane_uuid.clone()],
    );
    let mut snapshot = local_pane_snapshot_for_test(&pane_uuid.0, Some(directory.path()));
    snapshot.input_config = None;

    assert_eq!(
        resume_eligibility(
            &agent_restore,
            &pane_uuid,
            &snapshot,
            Some(directory.path())
        ),
        Err(ResumeIneligibility::SharedSessionViewer)
    );
}

// A tab restored from a snapshot mid-session reaches the same restore path as startup. Resuming
// there would relaunch an agent the user never lost.
#[test]
fn resume_is_ineligible_outside_the_startup_restore_pass() {
    let directory = tempfile::tempdir().expect("temp dir");
    let pane_uuid = PaneUuid(vec![1]);
    let agent_restore = AgentSessionRestore {
        is_startup_restore: false,
        ..startup_restore_for_test(
            [(
                pane_uuid.clone(),
                recorded_session_for_test("session-1", directory.path()),
            )],
            [pane_uuid.clone()],
        )
    };

    assert_eq!(
        resume_eligibility(
            &agent_restore,
            &pane_uuid,
            &local_pane_snapshot_for_test(&pane_uuid.0, Some(directory.path())),
            Some(directory.path())
        ),
        Err(ResumeIneligibility::NotStartupRestore)
    );
}

// U8 reports why a resume did not happen, which is only worth reporting if each rejection is its
// own reason: a gate that answered with one "not eligible" would make every cause look alike.
#[test]
fn every_resume_rejection_carries_its_own_reason() {
    let directory = tempfile::tempdir().expect("temp dir");
    let pane_uuid = PaneUuid(vec![1]);
    let unrecorded_pane = PaneUuid(vec![9]);
    let recorded = recorded_session_for_test("session-1", directory.path());
    let local_snapshot = local_pane_snapshot_for_test(&pane_uuid.0, Some(directory.path()));

    let mut viewer_snapshot = local_snapshot.clone();
    viewer_snapshot.input_config = None;
    let mut without_identifier = recorded.clone();
    without_identifier.session_id = String::new();
    let mut undeclared_agent = recorded.clone();
    undeclared_agent.agent = crate::terminal::CLIAgent::Gemini;
    let missing_directory = recorded_session_for_test("session-1", &directory.path().join("gone"));

    let claimed =
        startup_restore_for_test([(pane_uuid.clone(), recorded.clone())], [pane_uuid.clone()]);
    let reject = |restore: &AgentSessionRestore,
                  pane: &PaneUuid,
                  snapshot: &TerminalPaneSnapshot,
                  restored_directory: Option<&Path>| {
        resume_eligibility(restore, pane, snapshot, restored_directory)
            .expect_err("every case here should be rejected")
    };

    let reasons = vec![
        reject(&claimed, &unrecorded_pane, &local_snapshot, None),
        reject(
            &AgentSessionRestore {
                is_startup_restore: false,
                ..claimed.clone()
            },
            &pane_uuid,
            &local_snapshot,
            Some(directory.path()),
        ),
        reject(
            &startup_restore_for_test(
                [(pane_uuid.clone(), without_identifier)],
                [pane_uuid.clone()],
            ),
            &pane_uuid,
            &local_snapshot,
            Some(directory.path()),
        ),
        reject(
            &startup_restore_for_test([(pane_uuid.clone(), undeclared_agent)], [pane_uuid.clone()]),
            &pane_uuid,
            &local_snapshot,
            Some(directory.path()),
        ),
        reject(
            &claimed,
            &pane_uuid,
            &viewer_snapshot,
            Some(directory.path()),
        ),
        reject(
            &claimed,
            &pane_uuid,
            &local_pane_snapshot_for_test(&pane_uuid.0, None),
            None,
        ),
        reject(
            &startup_restore_for_test(
                [(pane_uuid.clone(), missing_directory)],
                [pane_uuid.clone()],
            ),
            &pane_uuid,
            &local_snapshot,
            Some(directory.path()),
        ),
        reject(&claimed, &pane_uuid, &local_snapshot, None),
        reject(
            &startup_restore_for_test([(pane_uuid.clone(), recorded)], []),
            &pane_uuid,
            &local_snapshot,
            Some(directory.path()),
        ),
    ];

    let distinct: HashSet<ResumeIneligibility> = reasons.iter().copied().collect();
    assert_eq!(
        distinct.len(),
        reasons.len(),
        "each rejection should report a different reason, got {reasons:?}"
    );
}

// R10: an ineligible pane restores exactly as it does today. Nothing about the restored pane may
// hint that a session was skipped, so the pane tree has to come back the way it does with no
// recording at all.
#[test]
fn an_ineligible_recorded_session_restores_the_pane_unchanged() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let pane_uuid = vec![4, 2];
        let agent_restore = startup_restore_for_test(
            [(
                PaneUuid(pane_uuid.clone()),
                // The pane snapshot carries no cwd and the recorded directory does not resolve,
                // so this recording cannot produce a resume however it is read.
                recorded_session_for_test("session-1", Path::new("/warp/no/such/directory")),
            )],
            [PaneUuid(pane_uuid.clone())],
        );

        let restored_panes = |app: &mut App, restore: AgentSessionRestore| {
            let layout = PanesLayout::Snapshot(Box::new(PaneNodeSnapshot::Leaf(LeafSnapshot {
                is_focused: true,
                custom_vertical_tabs_title: None,
                contents: LeafContents::Terminal(local_pane_snapshot_for_test(&pane_uuid, None)),
            })));
            let tips_model = app.add_model(|_| TipsCompleted::default());
            let (_, pane_group) = app.add_window_with_bounds(
                WindowStyle::NotStealFocus,
                WindowBounds::ExactPosition(RectF::new(
                    Vector2F::zero(),
                    Vector2F::new(1024., 768.),
                )),
                |ctx| {
                    let banner_model_handle = ctx.add_model(|_| BannerState::default());
                    PaneGroup::new_with_panes_layout(
                        tips_model,
                        banner_model_handle,
                        ServerApiProvider::as_ref(ctx).get(),
                        layout,
                        Arc::new(HashMap::new()),
                        restore,
                        None,
                        ctx,
                    )
                },
            );
            pane_group.read(app, |panes, _ctx| {
                panes
                    .panes_of::<TerminalPane>()
                    .map(|pane| pane.session_uuid())
                    .collect::<Vec<_>>()
            })
        };

        let with_ineligible_recording = restored_panes(&mut app, agent_restore);
        let without_recording = restored_panes(&mut app, AgentSessionRestore::default());

        assert_eq!(with_ineligible_recording, vec![pane_uuid]);
        assert_eq!(with_ineligible_recording, without_recording);
    });
}

/// Restores `panes` through the startup path with `restore` in force, and reports what each pane
/// came back with: its session uuid and the resume invocation armed for it.
fn restored_panes_with_armed_resume(
    app: &mut App,
    panes: Vec<TerminalPaneSnapshot>,
    restore: AgentSessionRestore,
) -> Vec<(Vec<u8>, Option<String>)> {
    let children = panes
        .into_iter()
        .map(|pane| {
            (
                crate::app_state::PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: false,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Terminal(pane),
                }),
            )
        })
        .collect();
    let layout = PanesLayout::Snapshot(Box::new(PaneNodeSnapshot::Branch(BranchSnapshot {
        direction: crate::app_state::SplitDirection::Horizontal,
        children,
    })));

    let tips_model = app.add_model(|_| TipsCompleted::default());
    let (_, pane_group) = app.add_window_with_bounds(
        WindowStyle::NotStealFocus,
        WindowBounds::ExactPosition(RectF::new(Vector2F::zero(), Vector2F::new(1024., 768.))),
        |ctx| {
            let banner_model_handle = ctx.add_model(|_| BannerState::default());
            PaneGroup::new_with_panes_layout(
                tips_model,
                banner_model_handle,
                ServerApiProvider::as_ref(ctx).get(),
                layout,
                Arc::new(HashMap::new()),
                restore,
                None,
                ctx,
            )
        },
    );

    let mut restored: Vec<_> = pane_group.read(app, |panes, ctx| {
        panes
            .panes_of::<TerminalPane>()
            .map(|pane| {
                let armed = pane.terminal_view(ctx).read(ctx, |view, _| {
                    view.armed_agent_session_resume().map(str::to_owned)
                });
                (pane.session_uuid(), armed)
            })
            .collect()
    });
    // Sorted by uuid: the restore walks the tree in whatever order it likes, and the question
    // here is which pane got which invocation, not which pane was built first.
    restored.sort_by(|(left, _), (right, _)| left.cmp(right));
    restored
}

/// AE7/R6: every eligible pane comes back carrying its own invocation, in the ordinary startup
/// restore, with no per-tab step and nothing for the user to do.
#[test]
fn every_eligible_restored_pane_carries_its_own_resume_invocation() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().to_path_buf();

    App::test((), |mut app| async move {
        let _resume_flag = FeatureFlag::AgentSessionResume.override_enabled(true);
        initialize_app(&mut app);

        let first = PaneUuid(vec![1]);
        let second = PaneUuid(vec![2]);
        let restore = startup_restore_for_test(
            [
                (first.clone(), recorded_session_for_test("session-1", &path)),
                (
                    second.clone(),
                    recorded_session_for_test("session-2", &path),
                ),
            ],
            [first.clone(), second.clone()],
        );

        let restored = restored_panes_with_armed_resume(
            &mut app,
            vec![
                local_pane_snapshot_for_test(&first.0, Some(&path)),
                local_pane_snapshot_for_test(&second.0, Some(&path)),
            ],
            restore,
        );

        assert_eq!(
            restored,
            vec![
                (
                    first.0.clone(),
                    Some(format!(
                        "claude --resume 'session-1' # {RESUME_HISTORY_MARKER}"
                    ))
                ),
                (
                    second.0.clone(),
                    Some(format!(
                        "claude --resume 'session-2' # {RESUME_HISTORY_MARKER}"
                    ))
                ),
            ]
        );
    });
}

/// With the feature off, an eligible pane restores exactly as it does today: a bare shell.
#[test]
fn no_resume_is_armed_while_the_feature_is_off() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().to_path_buf();

    App::test((), |mut app| async move {
        let _resume_flag = FeatureFlag::AgentSessionResume.override_enabled(false);
        initialize_app(&mut app);

        let pane = PaneUuid(vec![7]);
        let restore = startup_restore_for_test(
            [(pane.clone(), recorded_session_for_test("session-1", &path))],
            [pane.clone()],
        );

        let restored = restored_panes_with_armed_resume(
            &mut app,
            vec![local_pane_snapshot_for_test(&pane.0, Some(&path))],
            restore,
        );

        assert_eq!(restored, vec![(pane.0.clone(), None)]);
    });
}

/// R8: an eligible pane restores the same pane tree an unrecorded one does. The invocation is
/// something the pane runs on top of what it restored, not a different restore.
#[test]
fn an_eligible_pane_restores_the_same_way_an_unrecorded_one_does() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().to_path_buf();

    App::test((), |mut app| async move {
        let _resume_flag = FeatureFlag::AgentSessionResume.override_enabled(true);
        initialize_app(&mut app);

        let pane = PaneUuid(vec![9]);
        let snapshot = || vec![local_pane_snapshot_for_test(&pane.0, Some(&path))];

        let with_resume = restored_panes_with_armed_resume(
            &mut app,
            snapshot(),
            startup_restore_for_test(
                [(pane.clone(), recorded_session_for_test("session-1", &path))],
                [pane.clone()],
            ),
        );
        let without_recording =
            restored_panes_with_armed_resume(&mut app, snapshot(), AgentSessionRestore::default());

        assert_eq!(
            with_resume.iter().map(|(uuid, _)| uuid).collect::<Vec<_>>(),
            without_recording
                .iter()
                .map(|(uuid, _)| uuid)
                .collect::<Vec<_>>(),
            "the restored pane tree must not depend on whether a resume is armed"
        );
        assert!(with_resume[0].1.is_some());
        assert!(without_recording[0].1.is_none());
    });
}

/// The instant the resume-reporting tests restore at. Ages are expressed against it rather than
/// against the clock, so a band boundary is a value the test states.
fn resume_report_now() -> NaiveDateTime {
    chrono::NaiveDate::from_ymd_opt(2026, 8, 11)
        .expect("date should be valid")
        .and_hms_opt(12, 0, 0)
        .expect("time should be valid")
}

/// A recording of a Claude session last observed `age` before [`resume_report_now`].
fn recorded_session_observed_ago(age: chrono::Duration) -> RecordedAgentSession {
    RecordedAgentSession {
        observed_at: resume_report_now() - age,
        ..recorded_session_for_test("session-1", Path::new("/warp/recorded/directory"))
    }
}

/// The payload `outcome` produces for a pane whose state was observed `age` ago.
fn reported_resume_payload(age: chrono::Duration, outcome: ResumeOutcome) -> Value {
    let recorded = recorded_session_observed_ago(age);
    AgentSessionResumeTelemetryEvent::pane_restored(&recorded, outcome, resume_report_now())
        .payload()
        .expect("a reported resume outcome should carry a payload")
}

/// U8: the event the rollout gates count. A pane that came back with its session says so, and
/// says which agent it was running — the two values every other number is read against.
#[test]
fn a_resumed_pane_reports_its_agent_and_a_resumed_outcome() {
    let recorded = recorded_session_observed_ago(chrono::Duration::minutes(20));
    let outcome = ResumeOutcome::for_verdict(&Ok(&recorded), true)
        .expect("a pane that carried recorded state has an outcome to report");
    let event =
        AgentSessionResumeTelemetryEvent::pane_restored(&recorded, outcome, resume_report_now());

    assert_eq!(event.name(), "AgentSessionResume.PaneRestore.Outcome");
    assert_eq!(
        event.payload(),
        Some(serde_json::json!({
            "agent": "Claude",
            "outcome": "resumed",
            "permission_flags_carried": true,
            "recorded_age": "up_to_1h",
        }))
    );
}

/// R21: a pane the gate cleared that still armed nothing is the failure the rollout reads as
/// "resume is not reliable". It must not arrive looking like a pane that was never eligible.
#[test]
fn a_resume_that_armed_nothing_reports_a_failed_outcome() {
    let recorded = recorded_session_observed_ago(chrono::Duration::minutes(20));

    assert_eq!(
        ResumeOutcome::for_verdict(&Ok(&recorded), false),
        Some(ResumeOutcome::Failed)
    );
    assert_eq!(
        reported_resume_payload(chrono::Duration::minutes(20), ResumeOutcome::Failed)["outcome"],
        serde_json::json!("failed")
    );
}

/// U8: the outcomes are what a dashboard groups by, so each rejection has to arrive as its own
/// value — except the one that means "this pane was never running an agent", which is every
/// ordinary pane and says nothing about this feature.
#[test]
fn every_resume_rejection_reports_its_own_outcome() {
    // Spelled out rather than derived from the mapping under test: these strings are the wire
    // values a dashboard groups by, and the match stops a rejection added later from quietly
    // reaching the field without one.
    let expected_outcome = |reason| match reason {
        ResumeIneligibility::NoRecordedSession => None,
        ResumeIneligibility::NotStartupRestore => Some("not_startup_restore"),
        ResumeIneligibility::NoSessionIdentifier => Some("no_session_identifier"),
        ResumeIneligibility::AgentNotDeclared => Some("agent_not_declared"),
        ResumeIneligibility::SharedSessionViewer => Some("shared_session_viewer"),
        ResumeIneligibility::SessionNotLocal => Some("session_not_local"),
        ResumeIneligibility::RecordedDirectoryMissing => Some("recorded_directory_missing"),
        ResumeIneligibility::RestoredElsewhere => Some("restored_elsewhere"),
        ResumeIneligibility::IdentifierClaimedByAnotherPane => {
            Some("identifier_claimed_by_another_pane")
        }
    };
    let reasons = [
        ResumeIneligibility::NoRecordedSession,
        ResumeIneligibility::NotStartupRestore,
        ResumeIneligibility::NoSessionIdentifier,
        ResumeIneligibility::AgentNotDeclared,
        ResumeIneligibility::SharedSessionViewer,
        ResumeIneligibility::SessionNotLocal,
        ResumeIneligibility::RecordedDirectoryMissing,
        ResumeIneligibility::RestoredElsewhere,
        ResumeIneligibility::IdentifierClaimedByAnotherPane,
    ];

    for reason in reasons {
        let verdict: Result<&RecordedAgentSession, ResumeIneligibility> = Err(reason);
        let reported = ResumeOutcome::for_verdict(&verdict, false).map(|outcome| {
            serde_json::to_value(outcome).expect("an outcome should serialize as a plain value")
        });

        assert_eq!(
            reported,
            expected_outcome(reason).map(|expected| serde_json::json!(expected)),
            "{reason:?} should report its own outcome"
        );
    }

    let distinct: HashSet<&str> = reasons
        .iter()
        .filter_map(|reason| expected_outcome(*reason))
        .collect();
    assert_eq!(
        distinct.len(),
        reasons.len() - 1,
        "every rejection but the ordinary one should have a value of its own"
    );
}

/// R22: the elevation the user chose rides along only while the observation behind it is recent,
/// and whether it did is the half of the window's cost the age bands alone cannot show.
#[test]
fn a_resume_outside_the_freshness_window_reports_dropped_posture_flags() {
    let window = chrono::Duration::from_std(PERMISSION_POSTURE_FRESHNESS)
        .expect("the freshness window should fit a chrono duration");
    let carried =
        |age, outcome| reported_resume_payload(age, outcome)["permission_flags_carried"].clone();

    assert_eq!(
        carried(
            window - chrono::Duration::minutes(1),
            ResumeOutcome::Resumed
        ),
        serde_json::json!(true)
    );
    assert_eq!(
        carried(
            window + chrono::Duration::minutes(1),
            ResumeOutcome::Resumed
        ),
        serde_json::json!(false),
        "a recording older than the window resumes without the posture the user chose"
    );
    assert_eq!(
        carried(chrono::Duration::minutes(1), ResumeOutcome::Failed),
        serde_json::json!(false),
        "a pane that never launched carried nothing, however fresh its recording was"
    );
}

/// U8: the bands R22's window is chosen from. They bracket the candidate windows, so the field
/// distribution answers what moving the window to 6 or 24 hours would cost — the provisional 12
/// hours is a band edge rather than a band.
#[test]
fn a_resume_reports_the_recorded_age_in_bracketing_bands() {
    let bands = [
        (chrono::Duration::minutes(2), "up_to_1h"),
        (chrono::Duration::hours(1), "up_to_1h"),
        (chrono::Duration::hours(3), "1h_to_6h"),
        (chrono::Duration::hours(6), "1h_to_6h"),
        (chrono::Duration::hours(9), "6h_to_12h"),
        (chrono::Duration::hours(12), "6h_to_12h"),
        (chrono::Duration::hours(18), "12h_to_24h"),
        (chrono::Duration::hours(24), "12h_to_24h"),
        (chrono::Duration::days(3), "1d_to_7d"),
        (chrono::Duration::days(7), "1d_to_7d"),
        (chrono::Duration::days(30), "over_7d"),
        // A recording dated after the restart: the clock moved backwards, and no band can be
        // claimed for an age nothing vouches for.
        (chrono::Duration::minutes(-5), "unverifiable"),
    ];

    for (age, expected) in bands {
        assert_eq!(
            reported_resume_payload(age, ResumeOutcome::Resumed)["recorded_age"],
            serde_json::json!(expected),
            "state observed {age} before the restart belongs in {expected}"
        );
    }

    // The window sits on the 6h_to_12h edge, which is what makes the bands readable as a cost:
    // everything up to that edge is what carrying the posture flags currently covers.
    assert_eq!(
        RecordedAgeBucket::for_observation(
            resume_report_now()
                - chrono::Duration::from_std(PERMISSION_POSTURE_FRESHNESS)
                    .expect("the freshness window should fit a chrono duration"),
            resume_report_now(),
        ),
        RecordedAgeBucket::SixToTwelveHours
    );
}

/// R20: the event measures the feature without shipping any of what the user was doing. The
/// recorded state it is built from holds the session identifier, the flags off the user's own
/// command and a path on their disk, and none of the three may reach the payload.
#[test]
fn the_reported_resume_outcome_carries_nothing_of_the_session() {
    let recorded = RecordedAgentSession {
        agent: crate::terminal::CLIAgent::Claude,
        session_id: "SENSITIVE-session-id".to_owned(),
        flags: vec![RecordedFlag {
            name: "--SENSITIVE-flag".to_owned(),
            value: Some("SENSITIVE-flag-value".to_owned()),
        }],
        directory: PathBuf::from("/SENSITIVE/directory"),
        observed_at: resume_report_now() - chrono::Duration::hours(2),
    };

    let event = AgentSessionResumeTelemetryEvent::pane_restored(
        &recorded,
        ResumeOutcome::Resumed,
        resume_report_now(),
    );
    // Destructured exhaustively on purpose: a field added to the event has to be looked at here
    // before it can be reported.
    let AgentSessionResumeTelemetryEvent::PaneRestored {
        agent,
        outcome,
        permission_flags_carried,
        recorded_age,
    } = &event;
    let reported = format!("{agent:?} {outcome:?} {permission_flags_carried} {recorded_age:?}");
    let payload = event
        .payload()
        .expect("a reported resume outcome should carry a payload");
    let serialized = payload.to_string();

    for rendering in [&reported, &serialized] {
        assert!(
            !rendering.contains("SENSITIVE"),
            "the event must carry nothing of the recorded session, got: {rendering}"
        );
        assert!(
            !rendering.contains('/'),
            "the event must carry no path, got: {rendering}"
        );
        assert!(
            !rendering.contains("--"),
            "the event must carry no flag off the user's command, got: {rendering}"
        );
    }
    assert_eq!(
        payload
            .as_object()
            .expect("the payload should be an object")
            .keys()
            .collect::<Vec<_>>(),
        vec![
            "agent",
            "outcome",
            "permission_flags_carried",
            "recorded_age"
        ],
        "the payload is these four closed values and nothing else"
    );
    assert!(
        !event.contains_ugc(),
        "nothing the user generated reaches this event"
    );
}

/// The reporting is part of the feature, so it is behind the same flag: nothing about a restart
/// is measured where nothing about it is attempted.
#[test]
fn no_resume_outcome_is_reported_while_the_feature_is_off() {
    let event = AgentSessionResumeTelemetryEvent::pane_restored(
        &recorded_session_observed_ago(chrono::Duration::minutes(20)),
        ResumeOutcome::Resumed,
        resume_report_now(),
    );

    {
        let _resume_flag = FeatureFlag::AgentSessionResume.override_enabled(false);
        assert!(
            !event.enablement_state().is_enabled(),
            "the send path drops the event while the feature is off"
        );
    }

    let _resume_flag = FeatureFlag::AgentSessionResume.override_enabled(true);
    assert!(event.enablement_state().is_enabled());
}

/// Drains the resume outcomes recorded so far, waiting up to `wait` for `expected` of them: the
/// send hands the event to the background executor, so a drain taken the instant a pane restored
/// can be empty for reasons that have nothing to do with the pane.
async fn recorded_resume_outcomes(
    expected: usize,
    wait: std::time::Duration,
) -> Vec<(Option<Value>, bool)> {
    let deadline = instant::Instant::now() + wait;
    let mut recorded = Vec::new();
    loop {
        recorded.extend(
            warpui::telemetry::flush_events()
                .into_iter()
                .filter_map(|event| match event.payload {
                    EventPayload::NamedEvent { name, value, .. }
                        if name == "AgentSessionResume.PaneRestore.Outcome" =>
                    {
                        Some((value, event.contains_ugc))
                    }
                    _ => None,
                }),
        );
        if recorded.len() >= expected || instant::Instant::now() >= deadline {
            return recorded;
        }
        warpui::r#async::Timer::after(std::time::Duration::from_millis(10)).await;
    }
}

/// U8: the restore path itself reports, once per pane that carried recorded state — the event is
/// not a helper the path could be wired up without.
#[test]
fn a_restored_pane_reports_its_resume_outcome() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().to_path_buf();

    App::test((), |mut app| async move {
        let _resume_flag = FeatureFlag::AgentSessionResume.override_enabled(true);
        initialize_app(&mut app);
        warpui::telemetry::flush_events();

        let resuming_pane = PaneUuid(vec![1]);
        let ordinary_pane = PaneUuid(vec![2]);
        let recorded = RecordedAgentSession {
            // Observed as the restart happens, so the age band and the posture rule have a
            // definite answer here rather than one that depends on when the test runs.
            observed_at: Utc::now().naive_utc(),
            ..recorded_session_for_test("session-1", &path)
        };

        let restored = restored_panes_with_armed_resume(
            &mut app,
            vec![
                local_pane_snapshot_for_test(&resuming_pane.0, Some(&path)),
                local_pane_snapshot_for_test(&ordinary_pane.0, Some(&path)),
            ],
            startup_restore_for_test([(resuming_pane.clone(), recorded)], [resuming_pane.clone()]),
        );
        assert!(restored[0].1.is_some(), "the recorded pane should resume");

        let reported = recorded_resume_outcomes(1, std::time::Duration::from_secs(5)).await;

        assert_eq!(
            reported.len(),
            1,
            "the pane that carried recorded state should report once, got: {reported:?}"
        );
        assert!(
            recorded_resume_outcomes(1, std::time::Duration::from_millis(300))
                .await
                .is_empty(),
            "the pane that carried no recorded state was not running an agent and reports nothing"
        );
        assert_eq!(
            reported[0].0,
            Some(serde_json::json!({
                "agent": "Claude",
                "outcome": "resumed",
                "permission_flags_carried": true,
                "recorded_age": "up_to_1h",
            }))
        );
        assert!(!reported[0].1, "the event holds no user-generated content");
    });
}

/// With the feature off, a restart is not measured either: the pane restores as a bare shell and
/// says nothing about having been asked to resume.
#[test]
fn a_restored_pane_reports_no_resume_outcome_while_the_feature_is_off() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().to_path_buf();

    App::test((), |mut app| async move {
        let _resume_flag = FeatureFlag::AgentSessionResume.override_enabled(false);
        initialize_app(&mut app);
        warpui::telemetry::flush_events();

        let pane = PaneUuid(vec![1]);
        let restored = restored_panes_with_armed_resume(
            &mut app,
            vec![local_pane_snapshot_for_test(&pane.0, Some(&path))],
            startup_restore_for_test(
                [(pane.clone(), recorded_session_for_test("session-1", &path))],
                [pane.clone()],
            ),
        );
        assert_eq!(restored[0].1, None, "nothing should have been armed");

        let reported = recorded_resume_outcomes(1, std::time::Duration::from_millis(500)).await;

        assert!(
            reported.is_empty(),
            "the feature reports nothing while it is off, got: {reported:?}"
        );
    });
}
