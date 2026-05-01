use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::{
    ai::{
        active_agent_views_model::ActiveAgentViewsModel,
        agent_conversations_model::AgentConversationsModel,
        ambient_agents::github_auth_notifier::GitHubAuthNotifier,
        blocklist::BlocklistAIHistoryModel,
        document::ai_document_model::AIDocumentModel,
        execution_profiles::profiles::AIExecutionProfilesModel,
        llms::LLMPreferences,
        mcp::{
            templatable_manager::TemplatableMCPServerManager, FileBasedMCPManager, FileMCPWatcher,
        },
        outline::RepoOutlines,
        persisted_workspace::PersistedWorkspace,
        restored_conversations::RestoredAgentConversations,
        skills::SkillManager,
        AIRequestUsageModel,
    },
    auth::auth_manager::AuthManager,
    cloud_object::model::persistence::CloudModel,
    context_chips::prompt::Prompt,
    experiments,
    network::NetworkStatus,
    notebooks::{
        editor::keys::NotebookKeybindings, manager::NotebookManager, notebook::NotebookView,
    },
    pricing::PricingInfoModel,
    resource_center::TipsCompleted,
    search::files::model::FileSearchModel,
    server::{
        cloud_objects::{listener::Listener, update_manager::UpdateManager},
        server_api::ServerApiProvider,
        sync_queue::SyncQueue,
        telemetry::context_provider::AppTelemetryContextProvider,
    },
    settings::PrivacySettings,
    settings_view::keybindings::KeybindingChangedNotifier,
    suggestions::ignored_suggestions_model::IgnoredSuggestionsModel,
    system::SystemStats,
    terminal::history::History,
    terminal::{
        alt_screen_reporting::AltScreenReporting,
        keys::TerminalKeybindings,
        local_tty::{spawner::PtySpawner, TerminalManager},
        shared_session::{
            SharedSessionActionSource, SharedSessionScrollbackType, SharedSessionStatus,
        },
    },
    test_util::settings::initialize_settings_for_tests,
    undo_close::UndoCloseStack,
    warp_managed_paths_watcher::WarpManagedPathsWatcher,
    workflows::local_workflows::LocalWorkflows,
    workspace::{
        sync_inputs::SyncedInputState, ActiveSession, OneTimeModalModel, WorkspaceRegistry,
    },
    workspaces::{
        team_tester::TeamTesterStatus, update_manager::TeamUpdateManager,
        user_profiles::UserProfiles, user_workspaces::UserWorkspaces,
    },
    AgentNotificationsModel, GlobalResourceHandles, GlobalResourceHandlesProvider,
};
#[cfg(feature = "local_fs")]
use repo_metadata::RepoMetadataModel;
use repo_metadata::{repositories::DetectedRepositories, watcher::DirectoryWatcher};
use std::collections::HashMap;
use watcher::HomeDirectoryWatcher;

use super::*;
use crate::terminal::resizable_data::ResizableData;
use ai::{
    index::full_source_code_embedding::manager::CodebaseIndexManager,
    project_context::model::ProjectContextModel,
};
use pathfinder_geometry::rect::RectF;
use shared_session::permissions_manager::SessionPermissionsManager;
use warpui::windowing::{state::ApplicationStage, WindowManager};
use warpui::{
    platform::{WindowBounds, WindowStyle},
    App, ModelHandle,
};

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
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
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    app.add_singleton_model(|_| ActiveAgentViewsModel::new());
    app.add_singleton_model(AgentNotificationsModel::new);
    app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&crate::LaunchMode::new_for_unit_test(), ctx)
    });
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
    app.add_singleton_model(SessionPermissionsManager::new);
    app.add_singleton_model(LLMPreferences::new);
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
    #[cfg(feature = "local_fs")]
    app.add_singleton_model(RepoMetadataModel::new);
    app.add_singleton_model(SkillManager::new);
    app.add_singleton_model(FileSearchModel::new);
    app.add_singleton_model(|_| crate::code_review::git_status_update::GitStatusUpdateModel::new());
    app.add_singleton_model(RepoOutlines::new_for_test);
    crate::terminal::available_shells::register(app);
    app.update(experiments::init);
    AltScreenReporting::register(app);
    app.add_singleton_model(|ctx| {
        CodebaseIndexManager::new_for_test(ServerApiProvider::as_ref(ctx).get(), ctx)
    });
    app.add_singleton_model(|ctx| PersistedWorkspace::new(vec![], HashMap::new(), None, ctx));
    app.add_singleton_model(|_| ProjectContextModel::default());
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
            let initial_pane_count = panes.pane_count();
            let initial_visible_count = panes.visible_pane_count();
            let initial_active_session = panes.active_session_id(ctx);

            let child_pane_id = panes.insert_terminal_pane_hidden_for_child_agent(
                parent_pane_id,
                HashMap::new(),
                ctx,
            );

            assert_eq!(panes.pane_count(), initial_pane_count + 1);
            assert_eq!(panes.visible_pane_count(), initial_visible_count);
            assert!(panes.has_pane_id(child_pane_id.into()));

            // The new child pane should remain hidden and not affect visible ordering.
            assert_eq!(panes.pane_id_by_index(0), Some(parent_pane_id));
            assert_eq!(panes.pane_id_by_index(1), None);

            // Creating a hidden child pane should not steal focus or active session.
            assert_eq!(panes.focused_pane_id(ctx), parent_pane_id);
            assert_eq!(panes.active_session_id(ctx), initial_active_session);
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
            terminal_pane
                .terminal_manager(ctx)
                .update(ctx, |terminal_manager, ctx| {
                    let terminal_view = terminal_manager.view();
                    terminal_view.update(ctx, |terminal_view, ctx| {
                        terminal_view.attempt_to_share_session(
                            SharedSessionScrollbackType::None,
                            None,
                            SessionSourceType::default(),
                            false,
                            ctx,
                        );
                    });
                })
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
            terminal_pane
                .terminal_manager(ctx)
                .update(ctx, |terminal_manager, ctx| {
                    let terminal_view = terminal_manager.view();
                    terminal_view.update(ctx, |terminal_view, ctx| {
                        terminal_view
                            .stop_sharing_session(SharedSessionActionSource::PaneHeader, ctx);
                    });
                });
        });

        // Ensure the state is correct after stopping.
        pane_group.update(&mut app, |pane_group, ctx| {
            let terminal_pane = pane_group.terminal_session_by_pane_index(0).unwrap();
            let terminal_manager = terminal_pane
                .terminal_manager(ctx)
                .as_ref(ctx)
                .as_any()
                .downcast_ref::<TerminalManager>()
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
