use std::collections::HashMap;

use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use ai::project_context::model::ProjectContextModel;
use chrono::Utc;
use pathfinder_geometry::rect::RectF;
use persistence::model::{
    AgentConversation, AgentConversationData, AgentConversationRecord, ConversationUsageMetadata,
};
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
#[cfg(feature = "local_fs")]
use repo_metadata::RepoMetadataModel;
use session_sharing_protocol::common::SessionId;
use shared_session::permissions_manager::SessionPermissionsManager;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warp_server_client::iap::IapManager;
use warpui::platform::{WindowBounds, WindowStyle};
use warpui::windowing::state::ApplicationStage;
use warpui::windowing::WindowManager;
use warpui::{App, ModelHandle};
use watcher::HomeDirectoryWatcher;

use super::child_agent::hydration::{
    decide_remote_child_hydration_action, RemoteChildHydrationAction,
};
use super::child_agent::{
    create_hidden_child_agent_conversation, HiddenChildAgentConversationRequest,
    HiddenChildAgentTaskContext,
};
use super::*;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{
    AIAgentHarness, AIConversation, AIConversationId, ServerAIConversationMetadata,
};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier;
use crate::ai::ambient_agents::task::TaskPrincipalInfo;
use crate::ai::ambient_agents::{
    AgentSource, AmbientAgentLiveSessionState, AmbientAgentTask, AmbientAgentTaskId,
    AmbientAgentTaskState,
};
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::history_model::CloudConversationData;
use crate::ai::blocklist::local_agent_task_sync_model::LocalAgentTaskSyncModel;
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::orchestration_events::OrchestrationEventService;
use crate::ai::blocklist::orchestration_topology::descendant_conversation_ids_in_spawn_order;
use crate::ai::blocklist::{BlocklistAIHistoryModel, QueuedQueryModel};
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
use crate::ai::AIRequestUsageModel;
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
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::PrivacySettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::suggestions::ignored_suggestions_model::IgnoredSuggestionsModel;
use crate::system::SystemStats;
use crate::terminal::alt_screen_reporting::AltScreenReporting;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::history::History;
use crate::terminal::keys::TerminalKeybindings;
use crate::terminal::local_tty::spawner::PtySpawner;
use crate::terminal::local_tty::TerminalManager;
use crate::terminal::model::terminal_model::ConversationTranscriptViewerStatus;
use crate::terminal::resizable_data::ResizableData;
use crate::terminal::shared_session::{
    IsSharedSessionCreator, SharedSessionActionSource, SharedSessionScrollbackType,
    SharedSessionSource, SharedSessionStatus,
};
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
    experiments, AgentNotificationsModel, GlobalResourceHandles, GlobalResourceHandlesProvider,
};

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    // Disabled (`None`) IapManager so shared-session viewer code that reads the
    // singleton doesn't panic in tests; it is an inert no-op.
    app.add_singleton_model(|ctx| {
        IapManager::new(
            None,
            Box::new(|_| futures::FutureExt::boxed(futures::future::ready(None::<String>))),
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
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
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
    app.add_singleton_model(|_| RestoredAgentConversations::new(vec![]));
    app.add_singleton_model(OneTimeModalModel::new);
    app.add_singleton_model(|_| WorkspaceRegistry::new());
    app.add_singleton_model(UndoCloseStack::new);
    app.add_singleton_model(|_| IgnoredSuggestionsModel::new(vec![]));
    app.add_singleton_model(|_| PricingInfoModel::new());
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
    }
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
            credits_spent_for_last_block: None,
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
            assert_eq!(panes.visible_pane_count(), initial_visible_count);
            assert!(panes.has_pane_id(child_pane_id.into()));
            assert!(!panes.panes.is_pane_in_tree(child_pane_id.into()));

            // The new child pane should remain off-tree and not affect visible ordering.
            assert_eq!(panes.pane_id_by_index(0), Some(parent_pane_id));
            assert_eq!(panes.pane_id_by_index(1), None);

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

            // Fix B: inject mock cloud task data so the task-backed hydration
            // path resolves to the live ambient session via
            // `resolve_open_action` -> `OpenOrAttachAmbientAgentConversation`.
            // The default mock task has `is_sandbox_running = false` so it
            // resolves to the fallback path; we leave it at the default to
            // ensure the pre-Fix-B "attach to existing ambient session +
            // tombstone" behavior is preserved.
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(ambient_agent_task_for_current_user(task_id));
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

/// Fix B: when task data for a restored remote child is NOT yet cached at
/// `create_hidden_child_agent_pane` time, the placeholder must still be
/// registered in `child_agent_panes` keyed by its local AIConversationId,
/// attached to the live ambient session (preserving today's behavior so
/// streaming runs continue to attach), AND deferred via
/// `pending_remote_child_hydrations` so the subscription handler can retry
/// when `TasksUpdated` / `ConversationsLoaded` fires. The pane group must
/// never produce a worse state than the pre-Fix-B fallback.
#[test]
fn test_restored_remote_hidden_child_pane_fallback_when_task_data_unavailable() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let pane_group = mock_pane_group(&mut app, Default::default());

        pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = get_newly_created_pane_id(panes, &[]);
            let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
            let task_id = new_ambient_agent_task_id();

            // Deliberately do NOT inject a task into AgentConversationsModel.
            // `get_or_async_fetch_task_data` will return `None`, which forces
            // the hydration to:
            //   1. enter the existing ambient session in place (live-attach
            //      preserved per Fix B contract);
            //   2. register a pending hydration entry so the subscription
            //      handler can retry once task data lands.

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

            // Live-attach preserved: the ambient agent view model is
            // configured to view the existing session for `task_id`. This
            // matches the pre-Fix-B behavior so streaming runs continue to
            // attach.
            let (ambient_task_id, _is_agent_running, active_conversation_id) =
                ambient_child_session_state(panes, child_pane_id, ctx);
            assert_eq!(
                ambient_task_id,
                Some(task_id),
                "fallback path must still call enter_viewing_existing_session on the placeholder",
            );
            assert_eq!(active_conversation_id, Some(child_conversation_id));

            // Fix B: pending hydration is recorded so the subscription handler
            // can re-run hydration when task data lands. The map value is the
            // placeholder's local AIConversationId, which must match the
            // conversation we just restored.
            let pending_placeholder_id =
                panes.pending_remote_child_hydrations.get(&task_id).copied().expect(
                    "task-data-unavailable hydration must register a pending entry keyed by task id",
                );
            assert_eq!(
                pending_placeholder_id, child_conversation_id,
                "pending hydration must record the placeholder's local AIConversationId",
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
/// The disk-load construction path (`BlocklistAIHistoryModel::new(_, &conversations)`
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
                *store =
                    RestoredAgentConversations::new(vec![persisted_remote_child_conversation(
                        child_conversation_id,
                        Some(parent_conversation_id),
                        None,
                        task_id,
                    )]);
            });

            panes.restore_missing_child_agent_panes_for_parent(
                parent_conversation_id,
                parent_pane_id,
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

            assert!(!panes
                .child_agent_panes
                .contains_key(&local_child_conversation_id));

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

            assert!(!panes
                .child_agent_panes
                .contains_key(&remote_child_conversation_id));

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
fn test_ensure_hidden_child_agent_pane_materializes_restored_remote_child_linked_by_parent_agent_id(
) {
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
                *store =
                    RestoredAgentConversations::new(vec![persisted_remote_child_conversation(
                        child_conversation_id,
                        None,
                        Some(parent_run_id),
                        task_id,
                    )]);
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
                    .terminal_view_id_for_conversation(&child_conversation_id),
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
                    .terminal_view_id_for_conversation(&child_conversation_id),
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
            assert!(terminal_pane
                .pane_view()
                .as_ref(ctx)
                .header()
                .as_ref(ctx)
                .has_shareable_object(ctx));
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

            assert!(!terminal_pane
                .pane_view()
                .as_ref(ctx)
                .header()
                .as_ref(ctx)
                .has_shareable_object(ctx));
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
                assert!(pane_view
                    .as_ref(ctx)
                    .header()
                    .as_ref(ctx)
                    .is_visible_in_pane_group());
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
            assert!(pane_view
                .as_ref(ctx)
                .header()
                .as_ref(ctx)
                .is_visible_in_pane_group());
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
            assert!(pane_view
                .as_ref(ctx)
                .header()
                .as_ref(ctx)
                .is_visible_in_pane_group());
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
        loop {
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
            if all_terminals_bootstrapped {
                break;
            }
            // Return control back to the executor briefly so we can make
            // progress.
            futures_lite::future::yield_now().await;
        }

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

/// Builds an [`AmbientAgentTask`] tailored for unit-testing
/// [`decide_remote_child_hydration_action`].
///
/// `state`, `is_sandbox_running`, and `session_id` combine to determine the
/// task's [`AmbientAgentLiveSessionState`]:
/// - `state == InProgress`, `is_sandbox_running == true`, and a parseable
///   UUID-shaped `session_id` resolve to
///   [`AmbientAgentLiveSessionState::Attachable`].
/// - `state == InProgress`, `is_sandbox_running == true`, and an
///   unparseable `session_id` resolve to
///   [`AmbientAgentLiveSessionState::ActiveUnattachable`].
/// - Any other shape resolves to [`AmbientAgentLiveSessionState::Inactive`].
///
/// `conversation_id` populates the server conversation token used by the
/// `LoadTranscript` branch; pass `None` to exercise `Fallback`.
///
/// Note: `session_link` is unconditionally set to `None` in this helper.
/// `AmbientAgentTask::active_live_session_state` falls back to parsing the
/// session id out of `session_link` when `session_id` is absent, so a test
/// that exercises that branch would need a different helper.
fn hydration_decision_task(
    state: AmbientAgentTaskState,
    is_sandbox_running: bool,
    session_id: Option<&str>,
    conversation_id: Option<&str>,
) -> AmbientAgentTask {
    let mut task = ambient_agent_task_for_current_user(new_ambient_agent_task_id());
    task.state = state;
    task.is_sandbox_running = is_sandbox_running;
    task.session_id = session_id.map(str::to_string);
    task.session_link = None;
    task.conversation_id = conversation_id.map(str::to_string);
    task
}

#[test]
fn decide_remote_child_hydration_attachable_live_session_chooses_live_attach() {
    // InProgress + sandbox running + parseable session id -> Attachable.
    let task = hydration_decision_task(
        AmbientAgentTaskState::InProgress,
        true,
        Some("11111111-1111-1111-1111-111111111111"),
        Some("server-token-irrelevant-for-attach"),
    );
    assert_eq!(
        task.active_live_session_state(),
        AmbientAgentLiveSessionState::Attachable {
            session_id: "11111111-1111-1111-1111-111111111111".parse().unwrap(),
        },
    );

    assert_eq!(
        decide_remote_child_hydration_action(&task),
        RemoteChildHydrationAction::LiveAttach,
    );
}

#[test]
fn decide_remote_child_hydration_inactive_with_token_loads_transcript() {
    // Terminal state -> Inactive, server token present -> LoadTranscript.
    let task = hydration_decision_task(
        AmbientAgentTaskState::Succeeded,
        false,
        None,
        Some("my-server-token"),
    );
    assert_eq!(
        task.active_live_session_state(),
        AmbientAgentLiveSessionState::Inactive,
    );

    assert_eq!(
        decide_remote_child_hydration_action(&task),
        RemoteChildHydrationAction::LoadTranscript {
            server_token: ServerConversationToken::new("my-server-token".to_string()),
            task_is_terminal: true,
        },
    );
}

#[test]
fn decide_remote_child_hydration_active_unattachable_with_token_loads_transcript() {
    // InProgress + sandbox running + unparseable session id ->
    // ActiveUnattachable. With a server token we still prefer LoadTranscript
    // over Fallback so the user sees the merged transcript instead of a bare
    // tombstone.
    let task = hydration_decision_task(
        AmbientAgentTaskState::InProgress,
        true,
        Some("not-a-valid-uuid"),
        Some("unattachable-server-token"),
    );
    assert_eq!(
        task.active_live_session_state(),
        AmbientAgentLiveSessionState::ActiveUnattachable,
    );

    assert_eq!(
        decide_remote_child_hydration_action(&task),
        RemoteChildHydrationAction::LoadTranscript {
            server_token: ServerConversationToken::new("unattachable-server-token".to_string()),
            task_is_terminal: false,
        },
    );
}

#[test]
fn decide_remote_child_hydration_inactive_without_token_falls_back() {
    // Terminal state, no server token -> nothing to attach to and nothing to
    // load. Terminal => tombstone is appropriate.
    let task = hydration_decision_task(AmbientAgentTaskState::Succeeded, false, None, None);
    assert_eq!(
        task.active_live_session_state(),
        AmbientAgentLiveSessionState::Inactive,
    );

    assert_eq!(
        decide_remote_child_hydration_action(&task),
        RemoteChildHydrationAction::Fallback {
            task_is_terminal: true,
        },
    );
}

/// `ActiveUnattachable` + no server token: the run is still in progress but
/// the client can't attach and has nothing to load. Fallback must carry
/// `task_is_terminal: false` so the dispatch arm skips the
/// conversation-ended tombstone.
#[test]
fn decide_remote_child_hydration_active_unattachable_without_token_falls_back_non_terminal() {
    let task = hydration_decision_task(
        AmbientAgentTaskState::InProgress,
        true,
        Some("not-a-valid-uuid"),
        None,
    );
    assert_eq!(
        task.active_live_session_state(),
        AmbientAgentLiveSessionState::ActiveUnattachable,
    );

    assert_eq!(
        decide_remote_child_hydration_action(&task),
        RemoteChildHydrationAction::Fallback {
            task_is_terminal: false,
        },
    );
}

/// An `AmbientAgentTask` whose `conversation_id` is `Some("")` (or
/// whitespace-only) is treated the same as `None`: the dispatch must not
/// route to a no-op cloud fetch wrapped in a misleading tombstone. The
/// `Fallback` arm handles "nothing to attach to, nothing to load"
/// correctly. Terminal here => tombstone is appropriate.
#[test]
fn decide_remote_child_hydration_empty_token_falls_back() {
    for empty_token in [Some(""), Some("   "), Some("\t\n")] {
        let task =
            hydration_decision_task(AmbientAgentTaskState::Succeeded, false, None, empty_token);
        assert_eq!(
            task.active_live_session_state(),
            AmbientAgentLiveSessionState::Inactive,
            "empty/whitespace token={empty_token:?} should still resolve to Inactive",
        );
        assert_eq!(
            decide_remote_child_hydration_action(&task),
            RemoteChildHydrationAction::Fallback {
                task_is_terminal: true,
            },
            "empty/whitespace token={empty_token:?} must fall through to Fallback",
        );
    }
}
