use std::collections::HashMap;

use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use ai::project_context::model::ProjectContextModel;
use pane_group::{NotebookPane, PaneState, SplitPaneState, TerminalPaneId};
#[cfg(feature = "local_fs")]
use repo_metadata::CanonicalizedPath;
#[cfg(feature = "local_fs")]
use repo_metadata::RepoMetadataModel;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
use session_sharing_protocol::common::SessionId;
#[cfg(feature = "local_fs")]
use tempfile::TempDir;
use terminal::shared_session::permissions_manager::SessionPermissionsManager;
use terminal::view::ActiveSessionState;
use warp_editor::editor::NavigationKey;
#[cfg(feature = "local_fs")]
use warp_files::FileModel;
use warpui::platform::WindowStyle;
use warpui::{AddSingletonModel, App, ViewHandle};
use watcher::HomeDirectoryWatcher;

use super::*;
use crate::ai::AIRequestUsageModel;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::agent_tips::AITipModel;
use crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier;
use crate::ai::blocklist::agent_view::orchestration_pill_bar_model::OrchestrationPillBarModel;
use crate::ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions};
use crate::ai::document::ai_document_model::AIDocumentModel;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::facts::manager::AIFactManager;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::llms::LLMPreferences;
use crate::ai::mcp::gallery::MCPGalleryManager;
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::ai::mcp::{FileBasedMCPManager, FileMCPWatcher};
use crate::ai::outline::RepoOutlines;
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::ai::restored_conversations::RestoredAgentConversations;
use crate::ai::skills::SkillManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::model::view::CloudViewModel;
use crate::context_chips::prompt::Prompt;
use crate::editor::Event;
use crate::gpu_state::GPUState;
use crate::network::NetworkStatus;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::notebooks::notebook::NotebookView;
use crate::pane_group::{Direction, PaneGroupAction, PaneId};
use crate::pricing::PricingInfoModel;
#[cfg(not(target_family = "wasm"))]
use crate::remote_server::codebase_index_model::RemoteCodebaseIndexModel;
use crate::resource_center::Tip;
use crate::server::cloud_objects::listener::Listener;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::experiments::ServerExperiments;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::PrivacySettings;
use crate::settings::cloud_preferences_syncer::CloudPreferencesSyncer;
use crate::settings_view::DisplayCount;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::suggestions::ignored_suggestions_model::IgnoredSuggestionsModel;
use crate::system::SystemStats;
use crate::tab_configs::tab_config::{TabConfigPaneNode, TabConfigPaneType};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::history::History;
use crate::terminal::keys::TerminalKeybindings;
use crate::terminal::local_tty::spawner::PtySpawner;
use crate::terminal::shared_session::{
    SharedSessionScrollbackType, SharedSessionSource, SharedSessionStatus,
};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::undo_close::UndoCloseSettings;
#[cfg(feature = "local_fs")]
use crate::user_config::tab_configs_dir;
#[cfg(windows)]
use crate::util::traffic_lights::windows::RendererState;
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;
use crate::workflows::local_workflows::LocalWorkflows;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{
    AgentNotificationsModel, GlobalResourceHandlesProvider, ObjectActions, experiments, workspace,
};
pub(crate) fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    // Add the necessary singleton models to the App
    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|_ctx| PtySpawner::new_for_test());
    app.add_singleton_model(|_| Prompt::mock());
    app.add_singleton_model(|ctx| AutoupdateState::new(ServerApiProvider::as_ref(ctx).get()));
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(|_ctx| UserProfiles::new(Vec::new()));
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(TeamUpdateManager::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(MCPGalleryManager::new);
    app.add_singleton_model(CloudViewModel::mock);
    app.add_singleton_model(Listener::mock);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(AppearanceManager::new);
    app.add_singleton_model(|_| DisplayCount::mock());
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(|_ctx| RelaunchModel::new());
    app.add_singleton_model(|ctx| ChangelogModel::new(ServerApiProvider::as_ref(ctx).get()));
    app.add_singleton_model(|_| GitHubAuthNotifier::new());
    app.add_singleton_model(|_ctx| SyncedInputState::mock());
    app.add_singleton_model(|_| ResizableData::default());
    app.add_singleton_model(LocalWorkflows::new);
    app.add_singleton_model(UndoCloseStack::new);
    app.add_singleton_model(terminal::shared_session::manager::Manager::new);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(|_| WorkspaceToastStack);
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(TerminalKeybindings::new);
    app.add_singleton_model(NotebookManager::mock);
    app.add_singleton_model(|ctx| {
        CloudPreferencesSyncer::new(
            false,                     // force_local_wins_on_startup
            std::path::PathBuf::new(), // unused in tests that don't exercise the hash path
            true,                      // sync_enabled: cloud sync active in tests
            ctx,
        )
    });
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    // QueuedQueryModel subscribes to history events; register after the
    // history model is in place.
    app.add_singleton_model(crate::ai::blocklist::QueuedQueryModel::new);
    app.add_singleton_model(|ctx| OrchestrationPillBarModel::new(Default::default(), ctx));
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    // The blocklist controller created during terminal bootstrap subscribes to
    // OrchestrationEventService and OrchestrationEventStreamer unconditionally,
    // so both singletons must be registered before bootstrap.
    app.add_singleton_model(
        crate::ai::blocklist::orchestration_events::OrchestrationEventService::new,
    );
    app.add_singleton_model(
        crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer::new,
    );
    app.add_singleton_model(|_| ActiveAgentViewsModel::new());
    app.add_singleton_model(AgentNotificationsModel::new);
    app.add_singleton_model(AgentConversationsModel::new);
    app.add_singleton_model(SessionPermissionsManager::new);
    app.add_singleton_model(LLMPreferences::new);
    app.add_singleton_model(HarnessAvailabilityModel::new);
    app.add_singleton_model(|ctx| AITipModel::new_for_agent_tips(ctx));
    app.add_singleton_model(|_| SettingsPaneManager::new());
    app.add_singleton_model(|_| AIFactManager::new());

    // Initialize file-based MCP dependencies.
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
    app.add_singleton_model(DirectoryWatcher::new);
    app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
    app.add_singleton_model(FileMCPWatcher::new);
    app.add_singleton_model(|_| FileBasedMCPManager::default());

    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    #[cfg(feature = "local_fs")]
    app.add_singleton_model(FileModel::new);
    app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&crate::LaunchMode::new_for_unit_test(), ctx)
    });
    app.add_singleton_model(RepoOutlines::new_for_test);
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
    app.add_singleton_model(BlocklistAIPermissions::new);
    app.add_singleton_model(|_| GPUState::new());
    // Register IapManager in a disabled state (no IapState). The settings
    // page's `IapManager::as_ref(ctx).is_enabled()` check panics if the
    // singleton isn't registered, even though it's a no-op on production.
    app.add_singleton_model(|ctx| {
        warp_server_client::iap::IapManager::new(
            None,
            Box::new(|_| futures::FutureExt::boxed(futures::future::ready(None::<String>))),
            None,
            ctx,
        )
    });
    app.add_singleton_model(|_| RestoredAgentConversations::new_seeded(vec![]));
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
    app.add_singleton_model(OneTimeModalModel::new);
    // Register GlobalResourceHandlesProvider before ServerExperiments which depends on it
    let global_resource_handles = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
    app.add_singleton_model(|ctx| ServerExperiments::new_from_cache(vec![], ctx));
    app.add_singleton_model(DefaultTerminal::new);
    app.add_singleton_model(|_| IgnoredSuggestionsModel::new(vec![]));
    app.add_singleton_model(|_| crate::code_review::git_repo_model::GitRepoModels::new());
    app.add_singleton_model(remote_server::manager::RemoteServerManager::new);
    #[cfg(not(target_family = "wasm"))]
    app.add_singleton_model(RemoteCodebaseIndexModel::new);

    #[cfg(feature = "local_fs")]
    app.add_singleton_model(RepoMetadataModel::new);
    app.add_singleton_model(search::files::model::FileSearchModel::new);

    #[cfg(windows)]
    {
        app.add_singleton_model(RendererState::new);
    }

    #[cfg(feature = "local_tty")]
    terminal::available_shells::register(app);
    AltScreenReporting::register(app);

    #[cfg(enable_crash_recovery)]
    crate::crash_recovery::CrashRecovery::register_for_test(app);

    app.update(experiments::init);

    app.add_singleton_model(
        crate::workspace::bonus_grant_notification_model::BonusGrantNotificationModel::new,
    );
    app.add_singleton_model(|ctx| {
        CodebaseIndexManager::new_for_test(ServerApiProvider::as_ref(ctx).get(), ctx)
    });
    app.add_singleton_model(|ctx| PersistedWorkspace::new(vec![], HashMap::new(), None, ctx));
    app.add_singleton_model(|_| ProjectContextModel::default());
    app.add_singleton_model(|_| PricingInfoModel::new());
    app.add_singleton_model(AIDocumentModel::new);
    app.add_singleton_model(|_| History::new(vec![]));

    // SkillManager is registered after `HomeDirectoryWatcher`, `DirectoryWatcher`,
    // `WarpManagedPathsWatcher`, `DetectedRepositories`, and `RepoMetadataModel`
    // because `SkillWatcher::new` subscribes to all of them.
    app.add_singleton_model(SkillManager::new);

    // Make sure to initialize the keybindings so that they are available for subviews
    app.update(workspace::init);
}

pub(crate) fn mock_workspace(app: &mut App) -> ViewHandle<Workspace> {
    let global_resource_handles = GlobalResourceHandles::mock(app);
    let active_window_id = app.read(|ctx| ctx.windows().active_window());
    let (_, workspace) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        Workspace::new(
            global_resource_handles,
            None,
            NewWorkspaceSource::Empty {
                previous_active_window: active_window_id,
                shell: None,
            },
            ctx,
        )
    });
    workspace
}

fn restored_workspace(
    app: &mut App,
    window_snapshot: crate::app_state::WindowSnapshot,
) -> ViewHandle<Workspace> {
    let global_resource_handles = GlobalResourceHandles::mock(app);
    let (_, workspace) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        Workspace::new(
            global_resource_handles,
            None,
            NewWorkspaceSource::Restored {
                window_snapshot,
                block_lists: Arc::new(HashMap::new()),
            },
            ctx,
        )
    });
    workspace
}

fn transferred_tab_workspace(
    app: &mut App,
    vertical_tabs_panel_open: bool,
) -> ViewHandle<Workspace> {
    let global_resource_handles = GlobalResourceHandles::mock(app);
    let (_, workspace) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        Workspace::new(
            global_resource_handles,
            None,
            NewWorkspaceSource::TransferredTab {
                tab_color: None,
                custom_title: None,
                left_panel_open: false,
                vertical_tabs_panel_open,
                right_panel_open: false,
                is_right_panel_maximized: false,
                is_tab_drag_preview: false,
            },
            ctx,
        )
    });
    workspace
}

#[test]
fn test_tab_bar_traffic_light_space_regression_for_resource_center_overlap() {
    // Regression for #10139: the Resource Center/right panel can be open on
    // Windows/Linux, but vertical-tabs and right-panel state should not decide
    // whether the tab bar reserves space for titlebar controls.
    let cases = [
        (TrafficLightSide::Left, false),
        (TrafficLightSide::Right, true),
    ];

    for (side, should_reserve_space) in cases {
        assert_eq!(
            should_reserve_traffic_light_space_in_tab_bar(side),
            should_reserve_space
        );
    }
}

#[test]
fn test_theme_chooser_does_not_suppress_tab_bar_traffic_light_padding() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let closed_padding = workspace.compute_tab_bar_left_padding(ctx);
            assert!(
                closed_padding > 0.,
                "Tab bar should reserve left padding when no left panel is open"
            );

            workspace.current_workspace_state.is_theme_chooser_open = true;
            assert_eq!(
                workspace.compute_tab_bar_left_padding(ctx),
                closed_padding,
                "Theme chooser should not be treated as a left panel for tab bar padding"
            );

            workspace.open_left_panel(ctx);
            assert_eq!(
                workspace.compute_tab_bar_left_padding(ctx),
                closed_padding,
                "Open tools panel should still reserve tab bar traffic light padding"
            );
        });
    });
}

fn assert_vertical_tabs_tools_panel_preserves_padding(config: HeaderToolbarChipSelection) {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
                report_if_error!(
                    settings
                        .header_toolbar_chip_selection
                        .set_value(config, ctx)
                );
            });
        });

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let closed_padding = workspace.compute_tab_bar_left_padding(ctx);
            assert!(
                closed_padding > 0.,
                "Vertical tabs should reserve traffic light padding"
            );

            workspace.open_left_panel(ctx);
            assert_eq!(
                workspace.compute_tab_bar_left_padding(ctx),
                closed_padding,
                "An open tools panel should still reserve traffic light padding in vertical tabs"
            );
        });
    });
}

#[test]
fn test_tools_panel_does_not_suppress_vertical_tab_bar_traffic_light_padding() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    for config in [
        HeaderToolbarChipSelection::Custom {
            left: vec![HeaderToolbarItemKind::AgentManagement],
            right: vec![
                HeaderToolbarItemKind::TabsPanel,
                HeaderToolbarItemKind::ToolsPanel,
                HeaderToolbarItemKind::CodeReview,
                HeaderToolbarItemKind::NotificationsMailbox,
            ],
        },
        HeaderToolbarChipSelection::Custom {
            left: vec![
                HeaderToolbarItemKind::TabsPanel,
                HeaderToolbarItemKind::ToolsPanel,
                HeaderToolbarItemKind::AgentManagement,
            ],
            right: vec![
                HeaderToolbarItemKind::CodeReview,
                HeaderToolbarItemKind::NotificationsMailbox,
            ],
        },
    ] {
        assert_vertical_tabs_tools_panel_preserves_padding(config);
    }
}
/// Regression test for the handoff model carry-over: the copy must preserve
/// the source pane's explicit selection M even when M equals the destination
/// pane's current profile default — the case where re-normalizing the resolved
/// id against the destination's default (instead of copying the raw override)
/// would drop the override and let the source profile's default D win.
#[test]
fn copy_model_and_profile_preserves_explicit_model_over_source_profile_default() {
    use warpui::EntityId;

    use crate::ai::llms::{AvailableLLMs, LLMId, LLMInfo, ModelsByFeature};

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let m = LLMId::from("auto-genius");
        let d = LLMId::from("auto");

        let source_id = EntityId::new();
        let new_id = EntityId::new();

        app.update(|ctx| {
            // Catalog containing both slugs so profile/override ids resolve.
            LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                let models = ModelsByFeature {
                    agent_mode: AvailableLLMs::new(
                        "auto".into(),
                        vec![
                            LLMInfo::new_for_test("auto"),
                            LLMInfo::new_for_test("auto-genius"),
                        ],
                        None,
                    )
                    .expect("valid available llms"),
                    ..Default::default()
                };
                prefs.update_feature_model_choices(Ok(models), ctx);
            });

            // Default profile default = M (the destination pane's current profile).
            // Source uses a second profile whose default = D.
            let profiles = AIExecutionProfilesModel::handle(ctx);
            let default_profile_id = profiles.read(ctx, |p, _| p.default_profile_id());
            profiles.update(ctx, |p, ctx| {
                p.set_base_model(default_profile_id, Some(m.clone()), ctx);
                let source_profile_id = p.create_profile(ctx).expect("create source profile");
                p.set_base_model(source_profile_id, Some(d.clone()), ctx);
                p.set_active_profile(source_id, source_profile_id, ctx);
            });

            // Source's explicit selection = M (differs from its profile default D).
            LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                prefs.update_preferred_agent_mode_llm(&m, source_id, ctx);
            });
        });

        // Preconditions: source resolves to M; destination's current default is M.
        app.update(|ctx| {
            let prefs = LLMPreferences::as_ref(ctx);
            assert_eq!(
                prefs.get_active_base_model(ctx, Some(source_id)).id,
                m,
                "source pane should resolve to its explicit selection"
            );
            assert_eq!(
                prefs.get_active_base_model(ctx, Some(new_id)).id,
                m,
                "destination pane's current profile default should be M"
            );
        });

        // Carry source -> new via the production helper.
        app.update(|ctx| {
            Workspace::copy_model_and_profile_to_terminal_view(source_id, new_id, ctx);
        });

        app.update(|ctx| {
            assert_eq!(
                LLMPreferences::as_ref(ctx)
                    .get_active_base_model(ctx, Some(new_id))
                    .id,
                m,
                "destination pane must retain the source's explicit selection, not the source profile default"
            );
        });
    });
}

#[cfg(feature = "local_fs")]
fn open_worktree_sidecar(workspace: &ViewHandle<Workspace>, app: &mut App) {
    workspace.update(app, |workspace, ctx| {
        workspace.open_new_session_dropdown_menu(
            crate::workspace::action::NewSessionMenuAnchor::AddTabButton(Vector2F::zero()),
            ctx,
        );

        let worktree_index = workspace
            .new_session_dropdown_menu
            .read(ctx, |menu, _| {
                menu.items().iter().position(|item| {
                    matches!(
                        item,
                        MenuItem::Item(fields) if fields.label() == "New worktree config"
                    )
                })
            })
            .expect("expected new worktree config item in new-session menu");

        workspace
            .new_session_dropdown_menu
            .update(ctx, |menu, view_ctx| {
                menu.set_selected_by_index(worktree_index, view_ctx);
            });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_worktree_sidecar_hover_takes_precedence_over_selection() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let temp_root = TempDir::new().expect("failed to create temp dir");
        let alpha_repo = temp_root.path().join("alpha-repo");
        let beta_repo = temp_root.path().join("beta-repo");
        std::fs::create_dir_all(&alpha_repo).expect("failed to create alpha repo dir");
        std::fs::create_dir_all(&beta_repo).expect("failed to create beta repo dir");

        workspace.update(&mut app, |_, ctx| {
            PersistedWorkspace::handle(ctx).update(ctx, |persisted, ctx| {
                persisted.user_added_workspace(alpha_repo.clone(), ctx);
                persisted.user_added_workspace(beta_repo.clone(), ctx);
            });
        });

        open_worktree_sidecar(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace
                .new_session_sidecar_menu
                .update(ctx, |menu, view_ctx| {
                    menu.set_selected_by_index(1, view_ctx);
                    menu.handle_action(
                        &crate::menu::MenuAction::HoverSubmenuLeafNode {
                            depth: 0,
                            row_index: 2,
                            position: Vector2F::zero(),
                        },
                        view_ctx,
                    );
                });

            workspace.handle_new_session_sidecar_event(&MenuEvent::ItemHovered, ctx);
        });

        workspace.read(&app, |workspace, ctx| {
            assert_eq!(
                workspace
                    .new_session_sidecar_menu
                    .read(ctx, |menu, _| menu.selected_index()),
                Some(2)
            );
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_worktree_sidecar_pointer_entry_does_not_select_top_repo() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let temp_root = TempDir::new().expect("failed to create temp dir");
        let alpha_repo = temp_root.path().join("alpha-repo");
        let beta_repo = temp_root.path().join("beta-repo");
        std::fs::create_dir_all(&alpha_repo).expect("failed to create alpha repo dir");
        std::fs::create_dir_all(&beta_repo).expect("failed to create beta repo dir");

        workspace.update(&mut app, |_, ctx| {
            PersistedWorkspace::handle(ctx).update(ctx, |persisted, ctx| {
                persisted.user_added_workspace(alpha_repo.clone(), ctx);
                persisted.user_added_workspace(beta_repo.clone(), ctx);
            });
        });

        workspace.update(&mut app, |workspace, ctx| {
            workspace.open_new_session_dropdown_menu(
                crate::workspace::action::NewSessionMenuAnchor::AddTabButton(Vector2F::zero()),
                ctx,
            );

            let worktree_index = workspace
                .new_session_dropdown_menu
                .read(ctx, |menu, _| {
                    menu.items().iter().position(|item| {
                        matches!(
                            item,
                            MenuItem::Item(fields) if fields.label() == "New worktree config"
                        )
                    })
                })
                .expect("expected new worktree config item in new-session menu");

            workspace
                .new_session_dropdown_menu
                .update(ctx, |menu, view_ctx| {
                    menu.handle_action(
                        &crate::menu::MenuAction::HoverSubmenuWithChildren(
                            0,
                            crate::menu::SelectAction::Index {
                                row: worktree_index,
                                item: 0,
                            },
                        ),
                        view_ctx,
                    );
                });
            workspace.update_new_session_sidecar(ctx);
        });

        workspace.read(&app, |workspace, ctx| {
            assert!(workspace.show_new_session_sidecar);
            assert_eq!(
                workspace
                    .new_session_sidecar_menu
                    .read(ctx, |menu, _| menu.selected_index()),
                None
            );
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_worktree_sidecar_close_via_select_item_executes_from_workspace() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        let _cleanup = TabConfigCleanupGuard::new("alpha-repo");

        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let temp_root = TempDir::new().expect("failed to create temp dir");
        let alpha_repo = temp_root.path().join("alpha-repo");
        std::fs::create_dir_all(&alpha_repo).expect("failed to create alpha repo dir");

        workspace.update(&mut app, |_, ctx| {
            PersistedWorkspace::handle(ctx).update(ctx, |persisted, ctx| {
                persisted.user_added_workspace(alpha_repo.clone(), ctx);
            });
        });

        open_worktree_sidecar(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace
                .new_session_sidecar_menu
                .update(ctx, |menu, view_ctx| {
                    menu.set_selected_by_index(1, view_ctx);
                });
            workspace.handle_new_session_sidecar_event(
                &MenuEvent::Close {
                    via_select_item: true,
                },
                ctx,
            );
            workspace.handle_new_session_sidecar_event(&MenuEvent::ItemSelected, ctx);
        });

        workspace.read(&app, |workspace, _| {
            assert_eq!(workspace.tab_count(), 2);
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_open_file_notebook_focuses_existing_markdown_pane() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let markdown_path = temp_dir.path().join("README.md");
        std::fs::write(&markdown_path, "# Test\n").expect("failed to write markdown file");

        workspace.update(&mut app, |workspace, ctx| {
            workspace.open_file_with_target(
                markdown_path.clone(),
                FileTarget::MarkdownViewer(EditorLayout::SplitPane),
                None,
                CodeSource::Link {
                    path: markdown_path.clone(),
                    range_start: None,
                    range_end: None,
                },
                ctx,
            );
        });

        let markdown_pane_id = workspace.update(&mut app, |workspace, ctx| {
            let pane_group = workspace.active_tab_pane_group();
            pane_group.update(ctx, |pane_group, ctx| {
                let markdown_panes = pane_group.file_notebook_panes(ctx).collect_vec();
                assert_eq!(markdown_panes.len(), 1);
                let pane_id = markdown_panes[0].0;

                pane_group.add_terminal_pane(Direction::Right, None, ctx);
                assert_ne!(pane_group.focused_pane_id(ctx), pane_id);

                pane_id
            })
        });

        workspace.update(&mut app, |workspace, ctx| {
            workspace.open_file_with_target(
                markdown_path.clone(),
                FileTarget::MarkdownViewer(EditorLayout::SplitPane),
                None,
                CodeSource::Link {
                    path: markdown_path,
                    range_start: None,
                    range_end: None,
                },
                ctx,
            );
        });

        workspace.read(&app, |workspace, ctx| {
            let pane_group = workspace.active_tab_pane_group().as_ref(ctx);
            assert_eq!(pane_group.file_notebook_panes(ctx).count(), 1);
            assert_eq!(pane_group.focused_pane_id(ctx), markdown_pane_id);
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_worktree_sidecar_search_editor_enter_executes_selection() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        let _cleanup = TabConfigCleanupGuard::new("alpha-repo");

        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let temp_root = TempDir::new().expect("failed to create temp dir");
        let alpha_repo = temp_root.path().join("alpha-repo");
        std::fs::create_dir_all(&alpha_repo).expect("failed to create alpha repo dir");

        workspace.update(&mut app, |_, ctx| {
            PersistedWorkspace::handle(ctx).update(ctx, |persisted, ctx| {
                persisted.user_added_workspace(alpha_repo.clone(), ctx);
            });
        });

        open_worktree_sidecar(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace
                .worktree_sidecar_search_editor
                .update(ctx, |_, ctx| {
                    ctx.emit(Event::Enter);
                });
        });

        workspace.read(&app, |workspace, _| {
            assert_eq!(workspace.tab_count(), 2);
            assert!(workspace.show_new_session_dropdown_menu.is_none());
        });
    });
}

/// RAII guard that removes tab config TOML files whose name starts with
/// `prefix` from `~/.warp/tab_configs/` on drop. Because `Drop` runs even
/// when a test panics, this prevents stale worktree configs from leaking
/// into Warp dev.
#[cfg(feature = "local_fs")]
struct TabConfigCleanupGuard {
    prefix: &'static str,
}

#[cfg(feature = "local_fs")]
impl TabConfigCleanupGuard {
    fn new(prefix: &'static str) -> Self {
        // Eagerly clean up leftovers from any previously-crashed run.
        Self::clean(prefix);
        Self { prefix }
    }

    fn clean(prefix: &str) {
        let dir = tab_configs_dir();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(prefix) && name.ends_with(".toml") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

#[cfg(feature = "local_fs")]
impl Drop for TabConfigCleanupGuard {
    fn drop(&mut self) {
        Self::clean(self.prefix);
    }
}

/// Creates a workspace with a single, shared session.
fn mock_workspace_with_shared_session(app: &mut App) -> ViewHandle<Workspace> {
    use crate::terminal::shared_session::manager::Manager;

    // Create the workspace as a session-sharing sharer.
    let global_resource_handles = GlobalResourceHandles::mock(app);
    let (_, workspace) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        Workspace::new(
            global_resource_handles,
            None,
            NewWorkspaceSource::Empty {
                previous_active_window: None,
                shell: None,
            },
            ctx,
        )
    });

    // Get the single terminal view in the workspace.
    let terminal_view = workspace.read(app, |workspace, ctx| {
        assert_eq!(workspace.tabs.len(), 1);
        workspace
            .active_tab_pane_group()
            .as_ref(ctx)
            .focused_session_view(ctx)
            .unwrap()
    });

    terminal_view.update(app, |view, ctx| {
        view.model.lock().block_list_mut().set_bootstrapped();
        view.attempt_to_share_session(
            SharedSessionScrollbackType::All,
            None,
            SharedSessionSource::user(None),
            false,
            ctx,
        );
    });

    // Make sure the view is registered with the shared session manager.
    app.read(|ctx| {
        let manager = Manager::as_ref(ctx);
        let shared_sessions = manager.shared_views(ctx).collect_vec();
        assert_eq!(shared_sessions.len(), 1);
        assert_eq!(shared_sessions[0].id(), terminal_view.id());
    });

    workspace
}

// Creates a workspace as a viewer of a shared session.
fn mock_workspace_viewing_shared_session(app: &mut App) -> ViewHandle<Workspace> {
    // Create the workspace as a session-sharing sharer.
    let global_resource_handles = GlobalResourceHandles::mock(app);

    let session_id = SessionId::new();

    let (_, workspace) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        Workspace::new(
            global_resource_handles,
            None,
            NewWorkspaceSource::SharedSessionAsViewer { session_id },
            ctx,
        )
    });

    // Get the single terminal view in the workspace.
    let terminal_view = workspace.read(app, |workspace, ctx| {
        assert_eq!(workspace.tabs.len(), 1);
        workspace
            .active_tab_pane_group()
            .as_ref(ctx)
            .focused_session_view(ctx)
            .unwrap()
    });

    // Ensure session is opened as a viewer.
    terminal_view.read(app, |terminal, _ctx| {
        let model = terminal.model.clone();
        assert!(model.lock().shared_session_status().is_viewer());
    });

    workspace
}

/// Disable the warn-before-quit setting. Because we don't fully bootstrap the shell in tests, this
/// is generally needed in tests that close tabs.
fn disable_quit_warning(app: &mut AppContext) {
    GeneralSettings::handle(app).update(app, |settings, ctx| {
        settings
            .show_warning_before_quitting
            .set_value(false, ctx)
            .expect("Failed to disable quit warning");
    });
}

fn get_newly_created_pane_id(panes: &PaneGroup, existing_ids: &[PaneId]) -> PaneId {
    panes
        .pane_ids()
        .find(|id| !existing_ids.contains(id))
        .unwrap()
}

fn split_pane_state(
    panes: &PaneGroup,
    pane_id: impl Into<PaneId>,
    ctx: &AppContext,
) -> SplitPaneState {
    // Split pane state is now inferred from the pane group's focus state
    panes
        .focus_state_handle()
        .as_ref(ctx)
        .split_pane_state_for(pane_id.into())
}

fn active_session_state(
    panes: &PaneGroup,
    pane_id: TerminalPaneId,
    ctx: &AppContext,
) -> ActiveSessionState {
    if panes
        .terminal_view_from_pane_id(pane_id, ctx)
        .expect("Not a terminal pane")
        .as_ref(ctx)
        .is_active_session(ctx)
    {
        ActiveSessionState::Active
    } else {
        ActiveSessionState::Inactive
    }
}

#[test]
fn restore_conversation_in_active_pane_enters_existing_live_conversation_without_loading() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let terminal_view = workspace.read(&app, |workspace, ctx| {
            workspace
                .active_tab_pane_group()
                .as_ref(ctx)
                .focused_session_view(ctx)
                .expect("workspace should start with a terminal view")
        });
        let terminal_view_id = terminal_view.read(&app, |view, _| view.view_id());
        let conversation_id =
            BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
                history.start_new_conversation(terminal_view_id, false, false, false, ctx)
            });

        workspace.update(&mut app, |workspace, ctx| {
            assert_eq!(workspace.tab_count(), 1);

            workspace.restore_conversation_in_active_pane(conversation_id, ctx);

            assert_eq!(workspace.tab_count(), 1);
        });

        terminal_view.read(&app, |view, ctx| {
            assert_eq!(view.active_conversation_id(ctx), Some(conversation_id));
            assert_eq!(
                view.model.lock().conversation_transcript_viewer_status(),
                None
            );
        });
    });
}
fn new_session_menu_label(item: &MenuItem<WorkspaceAction>) -> String {
    match item {
        MenuItem::Item(fields) => fields.label().to_string(),
        MenuItem::Separator => "---".to_string(),
        MenuItem::ItemsRow { items } => items
            .iter()
            .map(|fields| fields.label().to_string())
            .collect::<Vec<_>>()
            .join(" | "),
        MenuItem::Submenu { fields, .. } => fields.label().to_string(),
        MenuItem::Header { fields, .. } => fields.label().to_string(),
    }
}

fn reopen_closed_session_menu_item(
    menu_items: &[MenuItem<WorkspaceAction>],
) -> &MenuItemFields<WorkspaceAction> {
    match menu_items.last() {
        Some(MenuItem::Item(fields)) if fields.label() == "Reopen closed session" => fields,
        _ => panic!("expected Reopen closed session to be the last new-session menu item"),
    }
}

#[test]
fn test_reward_modal_no_overlap() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        // Trigger the referral reward response
        workspace.update(&mut app, |view, ctx| {
            view.handle_referral_theme_status_event(
                &ReferralThemeEvent::SentReferralThemeActivated,
                ctx,
            );

            // This _should_ show the reward modal, since the changelog modal is _not_ active
            assert!(view.current_workspace_state.is_reward_modal_open);
        });
    });
}

#[test]
fn test_reward_modal_shows_for_received_referral() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |view, ctx| {
            view.handle_referral_theme_status_event(
                &ReferralThemeEvent::ReceivedReferralThemeActivated,
                ctx,
            );

            assert!(view.current_workspace_state.is_reward_modal_open);
        });
    });
}

#[test]
fn test_tab_renaming_editor_selections() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        // Add second tab and rename both of them to prepare for the test
        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.rename_tab_internal(0, "short_title", ctx);
            let selected_text = workspace
                .tab_rename_editor
                .read(ctx, |editor, ctx| editor.selected_text(ctx));
            assert_eq!("short_title", selected_text);

            // Ensure that whatever is selected, is the full title and not the leftover from
            // the previous, shorter one.
            workspace.rename_tab_internal(1, "very_long_title_this_is", ctx);
            let selected_text = workspace
                .tab_rename_editor
                .read(ctx, |editor, ctx| editor.selected_text(ctx));
            assert_eq!("very_long_title_this_is", selected_text);

            // Ensure that if we escape, the current editor's contents is going to be cleared
            // as well.
            workspace.handle_tab_rename_editor_event(&Event::Escape, ctx);
            let selected_text = workspace
                .tab_rename_editor
                .read(ctx, |editor, ctx| editor.selected_text(ctx));
            assert_eq!("", selected_text);
        });
    });
}

#[test]
fn test_tab_renaming_editor_reset() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.rename_tab_internal(0, "short_title", ctx);
            workspace.rename_tab_internal(1, "very_long_title_this_is", ctx);

            // Ensure that when the editor is initially not empty, it will be cleared before a user renames a tab
            workspace.tab_rename_editor.update(ctx, |editor, ctx| {
                editor.insert_selected_text("some-text", ctx);
            });
            workspace.rename_tab_internal(1, "new_very_long_title", ctx);
            let selected_text: String = workspace
                .tab_rename_editor
                .read(ctx, |editor, ctx| editor.selected_text(ctx));
            assert_eq!("new_very_long_title", selected_text);
        });
    });
}

#[test]
fn test_set_active_tab_name() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);

            workspace.handle_action(
                &WorkspaceAction::SetActiveTabName("  Backend API  ".to_string()),
                ctx,
            );
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .display_title(ctx),
                "Backend API"
            );
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .custom_title(ctx)
                    .as_deref(),
                Some("Backend API")
            );

            workspace.handle_action(&WorkspaceAction::ActivateTab(0), ctx);
            assert_ne!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .custom_title(ctx)
                    .as_deref(),
                Some("Backend API")
            );

            workspace.handle_action(&WorkspaceAction::ActivateTab(1), ctx);
            workspace.handle_action(&WorkspaceAction::SetActiveTabName("   ".to_string()), ctx);
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .custom_title(ctx)
                    .as_deref(),
                Some("Backend API")
            );
        });
    });
}

#[test]
fn test_set_active_tab_name_clears_active_rename_editor_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.rename_tab_internal(0, "old title", ctx);
            assert!(workspace.current_workspace_state.is_tab_being_renamed());

            workspace.handle_action(
                &WorkspaceAction::SetActiveTabName("new title".to_string()),
                ctx,
            );

            assert!(!workspace.current_workspace_state.is_tab_being_renamed());
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .display_title(ctx),
                "new title"
            );
        });
    });
}

#[test]
fn test_set_active_tab_color() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            let active = workspace.active_tab_index;

            // Setting a color stores it as the manual selection and resolves to it.
            workspace.handle_action(
                &WorkspaceAction::SetActiveTabColor(SelectedTabColor::Color(
                    AnsiColorIdentifier::Magenta,
                )),
                ctx,
            );
            assert_eq!(
                workspace.tabs[active].selected_color,
                SelectedTabColor::Color(AnsiColorIdentifier::Magenta),
            );
            assert_eq!(
                workspace.tabs[active].color(),
                Some(AnsiColorIdentifier::Magenta),
            );

            // Replacing with a different color overwrites the previous selection.
            workspace.handle_action(
                &WorkspaceAction::SetActiveTabColor(SelectedTabColor::Color(
                    AnsiColorIdentifier::Green,
                )),
                ctx,
            );
            assert_eq!(
                workspace.tabs[active].selected_color,
                SelectedTabColor::Color(AnsiColorIdentifier::Green),
            );

            // `Cleared` explicitly suppresses any color (including a directory default).
            workspace.handle_action(
                &WorkspaceAction::SetActiveTabColor(SelectedTabColor::Cleared),
                ctx,
            );
            assert_eq!(
                workspace.tabs[active].selected_color,
                SelectedTabColor::Cleared,
            );
            assert_eq!(workspace.tabs[active].color(), None);

            // `Unset` removes the manual override so a directory default could apply.
            // With no directory default configured, the resolved color is still `None`.
            workspace.handle_action(
                &WorkspaceAction::SetActiveTabColor(SelectedTabColor::Unset),
                ctx,
            );
            assert_eq!(
                workspace.tabs[active].selected_color,
                SelectedTabColor::Unset,
            );
            assert_eq!(workspace.tabs[active].color(), None);

            // Action targets the active tab — switching to tab 0 leaves the second tab
            // unaffected.
            workspace.handle_action(&WorkspaceAction::ActivateTab(0), ctx);
            workspace.handle_action(
                &WorkspaceAction::SetActiveTabColor(SelectedTabColor::Color(
                    AnsiColorIdentifier::Blue,
                )),
                ctx,
            );
            assert_eq!(
                workspace.tabs[0].selected_color,
                SelectedTabColor::Color(AnsiColorIdentifier::Blue),
            );
            assert_eq!(
                workspace.tabs[active].selected_color,
                SelectedTabColor::Unset,
            );
        });
    });
}

#[test]
fn test_workspace_sessions_retrieves_tabs() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let pane_id = workspace
                .get_pane_group_view(0)
                .map(|tab| tab.read(ctx, |tab, _ctx| tab.pane_id_by_index(0).unwrap()))
                .expect("WindowId was not retrieved.");

            assert!(
                workspace
                    .workspace_sessions(ctx.window_id(), ctx)
                    .any(|x| { x.pane_view_locator().pane_id == pane_id })
            );

            // Add a tab and check if workspace_sessions finds the second session from the new tab.
            workspace.add_terminal_tab(false, ctx);
            let new_pane_id = workspace
                .get_pane_group_view(1)
                .map(|tab| tab.read(ctx, |tab, _ctx| tab.pane_id_by_index(0).unwrap()))
                .expect("WindowId was not retrieved.");

            assert!(
                workspace
                    .workspace_sessions(ctx.window_id(), ctx)
                    .any(|x| { x.pane_view_locator().pane_id == new_pane_id })
            );
        });
    });
}

#[test]
fn test_workspace_sessions_retrieves_panes() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            // Add a new split pane to the right.
            if let Some(tab_view) = workspace.get_pane_group_view(0) {
                tab_view.update(ctx, |view, ctx| {
                    view.handle_action(&PaneGroupAction::Add(Direction::Right), ctx);
                })
            }

            // Get the EntityId of the new pane added to the current tab.
            let new_pane_id = workspace
                .get_pane_group_view(0)
                .map(|tab| tab.read(ctx, |tab, _ctx| tab.pane_id_by_index(1).unwrap()))
                .expect("WindowId was not retrieved.");
            assert!(
                workspace
                    .workspace_sessions(ctx.window_id(), ctx)
                    .any(|x| { x.pane_view_locator().pane_id == new_pane_id })
            );
        });
    });
}

fn number_of_shared_sessions_in_tab(
    workspace: &Workspace,
    index: usize,
    ctx: &AppContext,
) -> usize {
    workspace
        .get_pane_group_view(index)
        .map_or(0, |view| view.as_ref(ctx).number_of_shared_sessions(ctx))
}

/// Sets up the workspace with three tabs. The middle tab has two panes, where one is shared.
fn setup_session_sharing_test(workspace: &ViewHandle<Workspace>, app: &mut App) -> PaneId {
    let shared_pane_id = workspace.update(app, |workspace, ctx| {
        workspace.add_terminal_tab(false, ctx);
        workspace.add_terminal_tab(false, ctx);

        let tab_view = workspace.get_pane_group_view(1).unwrap();

        tab_view.update(ctx, |view, ctx| {
            assert_eq!(view.pane_count(), 1);
            view.focused_session_view(ctx)
                .unwrap()
                .update(ctx, |terminal, ctx| {
                    terminal.attempt_to_share_session(
                        SharedSessionScrollbackType::None,
                        None,
                        SharedSessionSource::user(None),
                        false,
                        ctx,
                    );
                });

            view.handle_action(&PaneGroupAction::Add(Direction::Right), ctx);
            assert_eq!(view.pane_count(), 2);

            view.pane_id_by_index(0).unwrap()
        })
    });

    workspace.read(app, |workspace, ctx| {
        assert_eq!(number_of_shared_sessions_in_tab(workspace, 1, ctx), 1);

        // Confirmation dialog starts not open.
        assert!(
            !workspace
                .current_workspace_state
                .is_close_session_confirmation_dialog_open
        );
    });

    shared_pane_id
}

#[test]
fn test_close_tab_confirmation_dialog() {
    let _guard = FeatureFlag::CreatingSharedSessions.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(disable_quit_warning);

        let workspace = mock_workspace(&mut app);
        setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let first_tab_id = workspace.get_pane_group_view(0).unwrap().id();

            // Trying to close tab with a shared pane opens dialog.
            workspace.handle_action(&WorkspaceAction::CloseTab(1), ctx);
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // User clicking cancel closes dialog.
            workspace.handle_close_session_confirmation_dialog_event(
                &CloseSessionConfirmationEvent::Cancel,
                ctx,
            );
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // Trying to close tab without a shared pane goes through without dialog.
            workspace.handle_action(&WorkspaceAction::CloseTab(2), ctx);
            assert_eq!(workspace.tab_count(), 2);
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // Close the tab with the shared pane.
            workspace.handle_action(&WorkspaceAction::CloseTab(1), ctx);
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            workspace.handle_close_session_confirmation_dialog_event(
                &CloseSessionConfirmationEvent::CloseSession {
                    dont_show_again: false,
                    open_confirmation_source: OpenDialogSource::CloseTab { tab_index: 1 },
                },
                ctx,
            );
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            assert_eq!(workspace.tab_count(), 1);
            assert_eq!(workspace.get_pane_group_view(0).unwrap().id(), first_tab_id);
        });
    });
}

#[test]
fn test_close_active_horizontal_tab_activates_tab_to_right() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(false, ctx));
            });
        });

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            let tab_to_right_id = workspace.get_pane_group_view(2).unwrap().id();

            workspace.activate_tab(1, ctx);
            workspace.close_tab(1, true, true, ctx);

            assert_eq!(workspace.tab_count(), 2);
            assert_eq!(workspace.active_tab_index(), 1);
            assert_eq!(workspace.active_tab_pane_group().id(), tab_to_right_id);
        });
    });
}

#[test]
fn test_close_last_horizontal_tab_activates_tab_to_left() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(false, ctx));
            });
        });

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            let tab_to_left_id = workspace.get_pane_group_view(1).unwrap().id();

            workspace.activate_tab(2, ctx);
            workspace.close_tab(2, true, true, ctx);

            assert_eq!(workspace.tab_count(), 2);
            assert_eq!(workspace.active_tab_index(), 1);
            assert_eq!(workspace.active_tab_pane_group().id(), tab_to_left_id);
        });
    });
}

#[test]
fn test_close_active_vertical_tab_activates_tab_below() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
        });

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            let tab_below_id = workspace.get_pane_group_view(2).unwrap().id();

            workspace.activate_tab(1, ctx);
            workspace.close_tab(1, true, true, ctx);

            assert_eq!(workspace.tab_count(), 2);
            assert_eq!(workspace.active_tab_index(), 1);
            assert_eq!(workspace.active_tab_pane_group().id(), tab_below_id);
        });
    });
}

#[test]
fn test_close_last_vertical_tab_activates_tab_above() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
        });

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            let tab_above_id = workspace.get_pane_group_view(1).unwrap().id();

            workspace.activate_tab(2, ctx);
            workspace.close_tab(2, true, true, ctx);

            assert_eq!(workspace.tab_count(), 2);
            assert_eq!(workspace.active_tab_index(), 1);
            assert_eq!(workspace.active_tab_pane_group().id(), tab_above_id);
        });
    });
}

#[test]
fn test_close_pane_confirmation_dialog() {
    let _guard = FeatureFlag::CreatingSharedSessions.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let shared_pane_id = setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let shared_pane_group_id = workspace.get_pane_group_view(1).unwrap().id();

            // User tries to close shared pane, dialog comes up.
            workspace.handle_file_tree_event(
                workspace.get_pane_group_view(1).unwrap().clone(),
                &pane_group::Event::CloseSharedSessionPaneRequested {
                    pane_id: shared_pane_id,
                },
                ctx,
            );
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // User confirms.
            workspace.handle_close_session_confirmation_dialog_event(
                &CloseSessionConfirmationEvent::CloseSession {
                    dont_show_again: false,
                    open_confirmation_source: OpenDialogSource::ClosePane {
                        pane_group_id: shared_pane_group_id,
                        pane_id: shared_pane_id,
                    },
                },
                ctx,
            );
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            assert_eq!(number_of_shared_sessions_in_tab(workspace, 1, ctx), 0);
            let remaining_pane_id = workspace
                .get_pane_group_view_with_id(shared_pane_group_id)
                .unwrap()
                .as_ref(ctx)
                .pane_id_by_index(0)
                .unwrap();
            assert_ne!(remaining_pane_id, shared_pane_id);
            assert_eq!(workspace.tab_count(), 3);
        });
    });
}

#[test]
fn test_reopen_closed_shared_tab() {
    let _guard = FeatureFlag::CreatingSharedSessions.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let shared_pane_group = workspace.get_pane_group_view(1).unwrap().clone();

            // Close the tab with the shared pane.
            workspace.close_tab(1, true, true, ctx);
            assert_eq!(workspace.tab_count(), 2);

            // Restore the shared tab.
            workspace.restore_closed_tab(1, TabData::new(shared_pane_group.to_owned()), ctx);
        });
        // Restored tab should no longer be shared.
        workspace.read(&app, |workspace, ctx| {
            let pane_group = workspace.get_pane_group_view(1).unwrap();
            assert!(!pane_group.as_ref(ctx).is_terminal_pane_being_shared(ctx));
            assert_eq!(workspace.tab_count(), 3);
        })
    });
}

#[test]
fn test_close_other_tabs_confirmation_dialog() {
    let _guard = FeatureFlag::CreatingSharedSessions.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let last_tab_id = workspace.get_pane_group_view(2).unwrap().id();

            // User tries to close other tabs choosing non-shared tab, dialog comes up.
            workspace.handle_action(&WorkspaceAction::CloseOtherTabs(2), ctx);
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // User confirms.
            workspace.handle_close_session_confirmation_dialog_event(
                &CloseSessionConfirmationEvent::CloseSession {
                    dont_show_again: false,
                    open_confirmation_source: OpenDialogSource::CloseOtherTabs { tab_index: 2 },
                },
                ctx,
            );
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            assert_eq!(workspace.tab_count(), 1);
            assert_eq!(workspace.get_pane_group_view(0).unwrap().id(), last_tab_id);
        });
    });
}

#[test]
fn test_close_tabs_right_confirmation_dialog() {
    let _guard = FeatureFlag::CreatingSharedSessions.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let first_tab_id = workspace.get_pane_group_view(0).unwrap().id();

            // User tries to close all tabs right of the left-most tab, dialog comes up.
            workspace.handle_action(&WorkspaceAction::CloseTabsRight(0), ctx);
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // User confirms.
            workspace.handle_close_session_confirmation_dialog_event(
                &CloseSessionConfirmationEvent::CloseSession {
                    dont_show_again: false,
                    open_confirmation_source: OpenDialogSource::CloseTabsDirection {
                        tab_index: 0,
                        direction: TabMovement::Right,
                    },
                },
                ctx,
            );
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            assert_eq!(workspace.tab_count(), 1);
            assert_eq!(workspace.get_pane_group_view(0).unwrap().id(), first_tab_id);
        });
    });
}

#[test]
fn test_confirmation_dialog_dont_show_again() {
    let _guard = FeatureFlag::CreatingSharedSessions.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(disable_quit_warning);

        let workspace = mock_workspace(&mut app);
        setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            // Close the tab with the shared pane, dialog comes up
            workspace.handle_action(&WorkspaceAction::CloseTab(1), ctx);
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // User confirms, checking "Don't show again".
            workspace.handle_close_session_confirmation_dialog_event(
                &CloseSessionConfirmationEvent::CloseSession {
                    dont_show_again: true,
                    open_confirmation_source: OpenDialogSource::CloseTab { tab_index: 1 },
                },
                ctx,
            );
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            assert_eq!(workspace.tab_count(), 2);

            // Share the first tab
            let tab_view = workspace.get_pane_group_view(0).unwrap();
            tab_view.update(ctx, |view, ctx| {
                view.terminal_manager(0, ctx)
                    .unwrap()
                    .as_ref(ctx)
                    .model()
                    .lock()
                    .set_shared_session_status(SharedSessionStatus::ActiveSharer);
            });

            // Close the shared tab. No dialog should come up and action should go through.
            workspace.handle_action(&WorkspaceAction::CloseActiveTab, ctx);
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            assert_eq!(workspace.tab_count(), 1);
        });
    });
}

#[test]
fn test_close_last_tab_skip_confirmation() {
    let _guard = FeatureFlag::CreatingSharedSessions.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(disable_quit_warning);

        let workspace = mock_workspace(&mut app);
        setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            // Close the non-shared tabs so there's just one shared tab left.
            workspace.handle_action(&WorkspaceAction::CloseTab(2), ctx);
            workspace.handle_action(&WorkspaceAction::CloseTab(0), ctx);
            assert_eq!(workspace.tab_count(), 1);
            // Close the last remaining tab with the shared pane, no dialog should come up because
            // we're going to close the window and there's already a confirmation on window close.
            workspace.handle_action(&WorkspaceAction::CloseActiveTab, ctx);
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
        });
    });
}

#[test]
fn test_notebook_pane_tracking() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            // Add a new notebook pane.
            workspace.open_notebook(
                &NotebookSource::New {
                    title: None,
                    owner: Owner::mock_current_user(),
                    initial_folder_id: None,
                },
                &OpenWarpDriveObjectSettings::default(),
                ctx,
                true,
            );

            // Get the ID of the new notebook.
            let pane_group = workspace
                .get_pane_group_view(0)
                .expect("Pane group does not exist")
                .clone();
            let notebook_view = pane_group
                .as_ref(ctx)
                .notebook_view_at_pane_index(0, ctx)
                .expect("Notebook view was not created")
                .clone();
            let notebook_pane_id = pane_group
                .as_ref(ctx)
                .pane_id_from_index(0)
                .expect("Notebook view should have been created");
            let notebook_id = notebook_view
                .as_ref(ctx)
                .notebook_id(ctx)
                .expect("Notebook should have an ID");

            // The notebook should be registered with the NotebookManager.
            let (window, locator) = NotebookManager::as_ref(ctx)
                .find_pane(&NotebookSource::Existing(notebook_id))
                .expect("Notebook pane should be registered");
            assert_eq!(window, ctx.window_id());
            assert_eq!(
                locator,
                PaneViewLocator {
                    pane_group_id: pane_group.id(),
                    pane_id: notebook_pane_id,
                }
            );

            // Re-opening the notebook should not create a new view.
            workspace.open_notebook(
                &NotebookSource::Existing(notebook_id),
                &OpenWarpDriveObjectSettings::default(),
                ctx,
                true,
            );
            assert_eq!(
                ctx.views_of_type::<NotebookView>(ctx.window_id()),
                Some(vec![notebook_view])
            );

            // Finally, closing the notebook pane should de-register it.
            pane_group.update(ctx, |pane_group, ctx| {
                pane_group.handle_action(&PaneGroupAction::RemoveActive, ctx)
            });
            assert_eq!(
                NotebookManager::handle(ctx)
                    .as_ref(ctx)
                    .find_pane(&NotebookSource::Existing(notebook_id)),
                None
            );
        });
    });
}

#[test]
fn test_set_active_terminal_input_contents_and_focus_app() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let initial_buffer_contents = workspace
                .get_active_input_view_handle(ctx)
                .map(|input_view_handle| input_view_handle.as_ref(ctx).buffer_text(ctx))
                .expect("There should be an active input view");
            assert_eq!(
                "", initial_buffer_contents,
                "initial active input should be empty"
            );

            workspace.set_active_terminal_input_contents_and_focus_app("foobar", ctx);

            assert_eq!(
                "foobar",
                workspace
                    .get_active_input_view_handle(ctx)
                    .map(|input_view_handle| input_view_handle.as_ref(ctx).buffer_text(ctx))
                    .expect("There should be an active input view")
            );
            assert!(ctx.windows().app_is_active());
        });
    });
}

/// Ensures that the terminal model is destroyed when it is no longer needed.
/// This is only a "workspace" test because we want to mimic what a normal
/// user would do and expect (e.g. close a tab and expect that its backing
/// data is correctly deallocated).
///
/// TODO(suraj): we may also want to investigate a more "real" integration test
/// that inspects the application process's overall memory consumption
/// instead of just the terminal model, but this is not easy because
/// 1. we want to measure non-shared memory (i.e. the "memory" value in Activity Monitor)
///    which is not easy; it's easier to measure "real memory" or RSS, but that includes
///    shared memory across processes.
/// 2. the test might be flaky depending on how much memory is actually allocated vs
///    freed up (not something easily controlled).
///
/// For now, this test is still useful because the terminal model is one of the largest data structures
/// maintained by our app, so we want to ensure we're not introducing regressions that cause it to not
/// be deallocated correctly.
#[test]
fn test_terminal_model_isnt_leaked() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Turn off undo-close so that we don't need to wait for deallocation.
        UndoCloseSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .enabled
                .set_value(false, ctx)
                .expect("Can turn off undo-close via settings.")
        });

        let workspace = mock_workspace(&mut app);

        let terminal_model = workspace.update(&mut app, |workspace, ctx| {
            // Add another tab so that the workspace isn't destroyed when we close the tab.
            workspace.add_terminal_tab(false, ctx);

            // Get a weak reference to the model.
            let model = workspace.get_active_session_terminal_model(ctx).unwrap();
            Arc::downgrade(&model)
        });

        workspace.update(&mut app, |workspace, ctx| {
            // Remove the tab. This should destroy the corresponding terminal view.
            workspace.remove_tab(workspace.active_tab_index(), true, true, ctx);
        });
        // For some reason, the update call above results in more pending effects, one of which
        // contains the actual logic that drops the `TerminalModel`.
        app.update(|_| ());

        // If we can't upgrade the weak reference, that means it was in fact destructed.
        assert!(
            terminal_model.upgrade().is_none(),
            "The terminal model should not exist once the tab is closed."
        )
    });
}

#[test]
fn test_open_or_toggle_warp_drive() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // First, unconditionally open Warp Drive as a system action. WD should be open and welcome tips should not have opening warp drive.
            workspace.open_or_toggle_warp_drive(
                false, /* toggle */
                false, /* explicit_user_action */
                ctx,
            );
            assert!(
                workspace.current_workspace_state.is_warp_drive_open,
                "Warp Drive should be open"
            );
            assert!(
                !workspace
                    .tips_completed
                    .as_ref(ctx)
                    .features_used
                    .contains(&Tip::Action(TipAction::OpenWarpDrive)),
                "Warp drive welcome tip should not be completed"
            );

            // Next, toggle warp drive as a user action. WD should be closed and tip should not be filled out.
            workspace.open_or_toggle_warp_drive(
                true, /* toggle */
                true, /* explicit_user_action */
                ctx,
            );
            assert!(
                !workspace.current_workspace_state.is_warp_drive_open,
                "Warp Drive should be closed"
            );
            assert!(
                !workspace
                    .tips_completed
                    .as_ref(ctx)
                    .features_used
                    .contains(&Tip::Action(TipAction::OpenWarpDrive)),
                "Warp drive welcome tip should not be completed"
            );

            // Finally, toggle warp drive again as a user action. WD should be open and tip filled out.
            workspace.open_or_toggle_warp_drive(
                true, /* toggle */
                true, /* explicit_user_action */
                ctx,
            );
            assert!(
                workspace.current_workspace_state.is_warp_drive_open,
                "Warp Drive should be open"
            );
            assert!(
                workspace
                    .tips_completed
                    .as_ref(ctx)
                    .features_used
                    .contains(&Tip::Action(TipAction::OpenWarpDrive)),
                "Warp drive welcome tip should not be completed"
            );
        });
    });
}

#[test]
fn test_stop_sharing_session() {
    use crate::terminal::shared_session::manager::Manager;
    let _guard = FeatureFlag::CreatingSharedSessions.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Create a workspace with a single session that's shared.
        let workspace = mock_workspace_with_shared_session(&mut app);
        let terminal_view = workspace.read(&app, |workspace, ctx| {
            assert_eq!(workspace.tabs.len(), 1);
            workspace
                .active_tab_pane_group()
                .as_ref(ctx)
                .focused_session_view(ctx)
                .unwrap()
        });

        // Stop sharing the shared session.
        workspace.update(&mut app, |workspace, ctx| {
            workspace.stop_sharing_session(
                &terminal_view.id(),
                SharedSessionActionSource::Tab,
                ctx,
            );
        });

        // Ensure that the session is no longer registered with the shared session manager.
        app.read(|ctx| {
            let manager = Manager::as_ref(ctx);
            let shared_sessions = manager.shared_views(ctx).collect_vec();
            assert_eq!(shared_sessions.len(), 0);
        });
    });
}

#[test]
fn test_stop_sharing_all_sessions_in_tab() {
    use crate::terminal::shared_session::manager::Manager;
    let _guard = FeatureFlag::CreatingSharedSessions.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Create a workspace with two tabs. First tab has two shared sessions. Second tab has one shared session.
        let workspace = mock_workspace_with_shared_session(&mut app);
        let second_tab_session = workspace.update(&mut app, |workspace, ctx| {
            workspace
                .active_tab_pane_group()
                .update(ctx, |pane_group, ctx| {
                    pane_group.handle_action(&PaneGroupAction::Add(Direction::Right), ctx);
                    pane_group
                        .terminal_view_at_pane_index(1, ctx)
                        .unwrap()
                        .update(ctx, |terminal_view, ctx| {
                            terminal_view.attempt_to_share_session(
                                SharedSessionScrollbackType::None,
                                None,
                                SharedSessionSource::user(None),
                                false,
                                ctx,
                            );
                        });
                });

            workspace.add_terminal_tab(false, ctx);
            workspace
                .active_tab_pane_group()
                .update(ctx, |pane_group, ctx| {
                    pane_group
                        .terminal_view_at_pane_index(0, ctx)
                        .unwrap()
                        .update(ctx, |terminal_view, ctx| {
                            terminal_view.attempt_to_share_session(
                                SharedSessionScrollbackType::None,
                                None,
                                SharedSessionSource::user(None),
                                false,
                                ctx,
                            );
                        });
                });

            workspace
                .active_tab_pane_group()
                .read(ctx, |pane_group, ctx| {
                    pane_group.terminal_view_at_pane_index(0, ctx).unwrap().id()
                })
        });

        // Ensure we have three shared sessions registered.
        app.read(|ctx| {
            let manager = Manager::as_ref(ctx);
            let shared_sessions = manager.shared_views(ctx).collect_vec();
            assert_eq!(shared_sessions.len(), 3);
        });

        // Stop sharing all sessions in first tab.
        workspace.update(&mut app, |workspace, ctx| {
            let tab = workspace.tabs[0].pane_group.downgrade();
            workspace.stop_sharing_all_panes_in_tab(&tab, ctx);
        });

        // Ensure that the only remaining shared session is the one in the other tab.
        app.read(|ctx| {
            let manager = Manager::as_ref(ctx);
            let shared_sessions = manager.shared_views(ctx).collect_vec();
            assert_eq!(shared_sessions.len(), 1);
            assert_eq!(shared_sessions[0].id(), second_tab_session);
        });
    });
}

#[test]
fn test_tab_context_menu_share_session_items() {
    let _guard = FeatureFlag::CreatingSharedSessions.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let shared_pane_id = setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            // Focus the shared session
            workspace.activate_tab(1, ctx);
            workspace
                .active_tab_pane_group()
                .update(ctx, |pane_group, ctx| {
                    pane_group.focus_pane_by_id(shared_pane_id, ctx);
                });
        });

        // When there's a single shared session in a tab (focused), the options
        // for sharing are "Stop sharing" and "Stop sharing all".
        workspace.read(&app, |workspace, ctx| {
            let items =
                workspace.tabs[1].menu_items(1, 3, &workspace.tab_groups, false, true, true, ctx);
            assert!(
                items[0].is_approximately_same_item_as(
                    &MenuItemFields::new("Stop sharing").into_item()
                )
            );
            assert!(items[1].is_approximately_same_item_as(
                &MenuItemFields::new("Stop sharing all").into_item()
            ));
        });

        // Focus the other, non-shared pane in the tab
        workspace.update(&mut app, |workspace, ctx| {
            workspace.activate_tab(1, ctx);
            workspace
                .active_tab_pane_group()
                .update(ctx, |pane_group, ctx| {
                    pane_group.pane_by_index(1).unwrap().focus(ctx);
                });
        });

        // When there's a single shared session in a tab (unfocused), the options
        // for sharing are "Share session" and "Stop sharing all".
        workspace.read(&app, |workspace, ctx| {
            let items =
                workspace.tabs[1].menu_items(1, 3, &workspace.tab_groups, false, true, true, ctx);
            assert!(
                items[0].is_approximately_same_item_as(
                    &MenuItemFields::new("Share session").into_item()
                )
            );
            assert!(items[1].is_approximately_same_item_as(
                &MenuItemFields::new("Stop sharing all").into_item()
            ));
        });

        // Stop sharing.
        workspace.update(&mut app, |workspace, ctx| {
            let tab = workspace.tabs[1].pane_group.downgrade();
            workspace.stop_sharing_all_panes_in_tab(&tab, ctx);
        });

        // When there's no shared sessions in a tab, the only option is "Share session".
        workspace.read(&app, |workspace, ctx| {
            let items =
                workspace.tabs[1].menu_items(1, 3, &workspace.tab_groups, false, true, true, ctx);
            assert!(
                items[0].is_approximately_same_item_as(
                    &MenuItemFields::new("Share session").into_item()
                )
            );
            assert!(items[1].is_approximately_same_item_as(&MenuItem::Separator));
        });
    });
}

#[test]
fn test_view_only_session() {
    let _guard = FeatureFlag::ViewingSharedSessions.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Trying to open command search
        let workspace = mock_workspace_viewing_shared_session(&mut app);
        workspace.update(&mut app, |workspace: &mut Workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::ShowCommandSearch(Default::default()), ctx);
        });

        // Ensure command search doesn't work for read-only shared sessions
        workspace.read(&app, |workspace, _ctx| {
            assert!(!workspace.current_workspace_state.is_command_search_open);
        });
    });
}

#[test]
// This tests the end-to-end behavior to correctly switch focus among panels.
// (The only panels that can be focused currently are WD, workspace, & the agent panel.)
fn test_switch_focus_panels() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |view, ctx| {
            view.focus_active_tab(ctx);
        });
        workspace.update(&mut app, |view, ctx| {
            assert!(
                view.active_tab_pane_group().is_self_or_child_focused(ctx),
                "Expected terminal to be focused"
            );
        });

        // Shift focus from terminal to left panel when WD is open
        workspace.update(&mut app, |view, ctx| {
            view.current_workspace_state.is_warp_drive_open = true;
            view.focus_left_panel(ctx);
        });
        workspace.update(&mut app, |view, ctx| {
            assert!(
                view.left_panel_view.is_self_or_child_focused(ctx),
                "Expected Warp Drive panel to be focused"
            );
        });

        // Shift focus from WD to left panel when AI panel is open
        workspace.update(&mut app, |view, ctx| {
            view.current_workspace_state.is_ai_assistant_panel_open = true;
            view.focus_left_panel(ctx);
        });
        workspace.update(&mut app, |view, ctx| {
            assert!(
                view.ai_assistant_panel.is_self_or_child_focused(ctx),
                "Expected AI panel to be focused"
            );
        });

        // Shift focus from AI panel to left panel (terminal)
        workspace.update(&mut app, |view, ctx| {
            view.focus_left_panel(ctx);
        });
        workspace.update(&mut app, |_view, ctx| {
            assert!(
                workspace.is_self_or_child_focused(ctx),
                "Expected terminal to be focused"
            );
        });

        // Shift focus from workspace to right panel when the agent panel is open
        workspace.update(&mut app, |view, ctx| {
            view.current_workspace_state.is_ai_assistant_panel_open = true;
            view.focus_right_panel(ctx);
        });
        workspace.update(&mut app, |view, ctx| {
            assert!(
                view.ai_assistant_panel.is_self_or_child_focused(ctx),
                "Expected AI panel to be focused"
            );
        });

        // Shift focus from WD to right panel (terminal)
        workspace.update(&mut app, |view, ctx| {
            view.focus_right_panel(ctx);
        });
        workspace.update(&mut app, |_view, ctx| {
            assert!(
                workspace.is_self_or_child_focused(ctx),
                "Expected terminal to be focused"
            );
        });
    });
}

#[test]
fn test_focus_notebook() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let pane_group = workspace.read(&app, |workspace, _ctx| {
            workspace
                .get_pane_group_view(0)
                .expect("should have pane group for tab 0")
                .clone()
        });

        let first_terminal_id = pane_group.read(&app, |panes, _ctx| {
            get_newly_created_pane_id(panes, &[])
                .as_terminal_pane_id()
                .expect("should be a terminal pane")
        });

        let notebook_id = pane_group.update(&mut app, |panes, ctx| {
            // Add a notebook to the left.
            let notebook_view = ctx.add_typed_action_view(NotebookView::new);
            panes.add_pane_with_direction(
                Direction::Left,
                NotebookPane::new(notebook_view, ctx),
                true, /* focus_new_pane */
                ctx,
            );
            get_newly_created_pane_id(panes, &[first_terminal_id.into()])
        });

        // The new pane should be focused, but the terminal is still the active session.
        pane_group.read(&app, |panes, ctx| {
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(panes.active_session_id(ctx), Some(first_terminal_id));
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert_eq!(
                active_session_state(panes, first_terminal_id, ctx),
                ActiveSessionState::Active
            );
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Focused)
            );
        });

        // Add a terminal below.
        let second_terminal_id = pane_group.update(&mut app, |panes, ctx| {
            panes.add_terminal_pane(Direction::Down, None, ctx);
            get_newly_created_pane_id(panes, &[first_terminal_id.into(), notebook_id])
                .as_terminal_pane_id()
                .expect("should be a terminal pane")
        });

        // The new terminal should be both focused and the active session.
        pane_group.read(&app, |panes, ctx| {
            assert_eq!(panes.focused_pane_id(ctx), second_terminal_id.into());
            assert_eq!(panes.active_session_id(ctx), Some(second_terminal_id));
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert_eq!(
                active_session_state(panes, first_terminal_id, ctx),
                ActiveSessionState::Inactive
            );
            assert_eq!(
                split_pane_state(panes, second_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Focused)
            );
            assert_eq!(
                active_session_state(panes, second_terminal_id, ctx),
                ActiveSessionState::Active
            );
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
        });

        // Close the new terminal.
        pane_group.update(&mut app, |panes, ctx| {
            panes.close_pane(second_terminal_id.into(), ctx);
        });

        // Focus should switch to the notebook, and the first terminal session
        // will activate.
        pane_group.read(&app, |panes, ctx| {
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(panes.active_session_id(ctx), Some(first_terminal_id));
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Focused)
            );
            assert_eq!(
                active_session_state(panes, first_terminal_id, ctx),
                ActiveSessionState::Active
            );
        });
    })
}

#[test]
fn test_close_active_session() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let pane_group = workspace.read(&app, |workspace, _ctx| {
            workspace
                .get_pane_group_view(0)
                .expect("should have pane group for tab 0")
                .clone()
        });

        let first_terminal_id = pane_group.read(&app, |panes, _ctx| {
            get_newly_created_pane_id(panes, &[])
                .as_terminal_pane_id()
                .expect("should be a terminal pane")
        });

        // Add a terminal above.
        let second_terminal_id = pane_group.update(&mut app, |panes, ctx| {
            panes.add_terminal_pane(Direction::Up, None, ctx);
            get_newly_created_pane_id(panes, &[first_terminal_id.into()])
                .as_terminal_pane_id()
                .expect("should be a terminal pane")
        });

        let notebook_id = pane_group.update(&mut app, |panes, ctx| {
            // Add a notebook to the left.
            let notebook_view = ctx.add_typed_action_view(NotebookView::new);
            panes.add_pane_with_direction(
                Direction::Left,
                NotebookPane::new(notebook_view, ctx),
                true, /* focus_new_pane */
                ctx,
            );
            get_newly_created_pane_id(
                panes,
                &[first_terminal_id.into(), second_terminal_id.into()],
            )
        });

        pane_group.read(&app, |panes, ctx| {
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(panes.active_session_id(ctx), Some(second_terminal_id));
        });

        pane_group.update(&mut app, |panes, ctx| {
            // Close the active session, which should leave the notebook focused and activate the
            // remaining session.
            panes.close_pane(second_terminal_id.into(), ctx);
        });

        pane_group.read(&app, |panes, ctx| {
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(panes.active_session_id(ctx), Some(first_terminal_id));
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert_eq!(
                active_session_state(panes, first_terminal_id, ctx),
                ActiveSessionState::Active
            );
        });

        pane_group.update(&mut app, |panes, ctx| {
            // Now, focus the remaining session, which should keep it activated.
            panes.focus_pane_by_id(first_terminal_id.into(), ctx);
        });

        pane_group.read(&app, |panes, ctx| {
            assert_eq!(panes.focused_pane_id(ctx), first_terminal_id.into());
            assert_eq!(panes.active_session_id(ctx), Some(first_terminal_id));
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Focused)
            );
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert_eq!(
                active_session_state(panes, first_terminal_id, ctx),
                ActiveSessionState::Active
            );
        });
    });
}

fn set_left_panel_visibility_across_tabs(is_enabled: bool, ctx: &mut ViewContext<Workspace>) {
    WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
        window_settings
            .left_panel_visibility_across_tabs
            .set_value(is_enabled, ctx)
            .expect("Failed to update left_panel_visibility_across_tabs setting");
    });
}

fn add_get_started_tab(workspace: &mut Workspace, ctx: &mut ViewContext<Workspace>) {
    workspace.add_tab_with_pane_layout(
        PanesLayout::Snapshot(Box::new(PaneNodeSnapshot::Leaf(LeafSnapshot {
            is_focused: true,
            custom_vertical_tabs_title: None,
            contents: LeafContents::GetStarted,
        }))),
        Arc::new(HashMap::<PaneUuid, Vec<SerializedBlockListItem>>::new()),
        None,
        ctx,
    );
}

fn find_terminal_tab_index(workspace: &Workspace, ctx: &AppContext) -> usize {
    workspace
        .tabs
        .iter()
        .position(|tab| tab.pane_group.as_ref(ctx).has_terminal_panes())
        .expect("Expected a terminal tab")
}

fn find_non_following_tab_index(workspace: &Workspace, ctx: &AppContext) -> usize {
    workspace
        .tabs
        .iter()
        .position(|tab| {
            !Workspace::should_enable_file_tree_and_global_search_for_pane_group(
                tab.pane_group.as_ref(ctx),
            )
        })
        .expect("Expected a non-following tab")
}

#[test]
fn test_left_panel_window_scoped_reconciles_between_terminal_tabs_when_enabled() {
    let _conversation_list_guard =
        FeatureFlag::AgentViewConversationListView.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            set_left_panel_visibility_across_tabs(true, ctx);

            workspace.add_terminal_tab(false, ctx);

            workspace.activate_tab(0, ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(!workspace.left_panel_open);

            workspace.open_left_panel(ctx);
            assert!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(workspace.left_panel_open);

            workspace.activate_tab(1, ctx);
            assert!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );

            workspace.close_left_panel(ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(!workspace.left_panel_open);

            workspace.activate_tab(0, ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
        });
    });
}

#[test]
fn test_left_panel_window_scoped_non_following_tab_does_not_reconcile_but_updates_window_state() {
    let _conversation_list_guard =
        FeatureFlag::AgentViewConversationListView.override_enabled(false);
    let _get_started_guard = FeatureFlag::GetStartedTab.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            set_left_panel_visibility_across_tabs(true, ctx);

            // Establish window-scoped desired state = open on a terminal tab.
            workspace.open_left_panel(ctx);
            assert!(workspace.left_panel_open);

            // Create a non-following tab (e.g. Get Started), which should not auto-open even though
            // the window state is open.
            add_get_started_tab(workspace, ctx);
            let non_following_tab_index = find_non_following_tab_index(workspace, ctx);
            workspace.activate_tab(non_following_tab_index, ctx);

            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(workspace.left_panel_open);

            // User actions in the non-following tab still update window state.
            workspace.open_left_panel(ctx);
            assert!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(workspace.left_panel_open);

            workspace.close_left_panel(ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(!workspace.left_panel_open);

            // The window state should reconcile back onto following tabs.
            let terminal_tab_index = find_terminal_tab_index(workspace, ctx);
            workspace.activate_tab(terminal_tab_index, ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );

            // But toggling the window state from a following tab should not auto-open the
            // non-following tab.
            workspace.open_left_panel(ctx);
            assert!(workspace.left_panel_open);

            workspace.activate_tab(non_following_tab_index, ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(workspace.left_panel_open);
        });
    });
}

#[test]
fn test_left_panel_window_scoped_disabled_keeps_per_tab_state() {
    let _conversation_list_guard =
        FeatureFlag::AgentViewConversationListView.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            set_left_panel_visibility_across_tabs(false, ctx);

            workspace.add_terminal_tab(false, ctx);

            // Open left panel on tab 0.
            workspace.activate_tab(0, ctx);
            workspace.open_left_panel(ctx);
            assert!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );

            // With window scoping disabled, switching tabs should not reconcile the open state.
            workspace.activate_tab(1, ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );

            // Each tab can be toggled independently.
            workspace.open_left_panel(ctx);
            assert!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );

            workspace.activate_tab(0, ctx);
            assert!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
        });
    });
}

#[test]
fn test_vertical_tabs_panel_visibility_restores_from_window_snapshot() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
        });

        let workspace = mock_workspace(&mut app);

        let closed_snapshot = workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = false;
            workspace.snapshot(ctx.window_id(), false, ctx)
        });
        let open_snapshot = workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = true;
            workspace.snapshot(ctx.window_id(), false, ctx)
        });

        let restored_closed = restored_workspace(&mut app, closed_snapshot);
        let restored_open = restored_workspace(&mut app, open_snapshot);

        restored_closed.read(&app, |workspace, _| {
            assert!(!workspace.vertical_tabs_panel_open);
        });
        restored_open.read(&app, |workspace, _| {
            assert!(workspace.vertical_tabs_panel_open);
        });
    });
}

#[test]
fn test_open_vertical_tabs_panel_is_idempotent() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = false;
            workspace.handle_action(&WorkspaceAction::OpenVerticalTabsPanel, ctx);
            assert!(workspace.vertical_tabs_panel_open);

            workspace.handle_action(&WorkspaceAction::OpenVerticalTabsPanel, ctx);
            assert!(workspace.vertical_tabs_panel_open);
        });
    });
}

#[test]
fn test_vertical_tabs_panel_restored_open_when_show_in_restored_windows_enabled() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
                report_if_error!(
                    settings
                        .show_vertical_tab_panel_in_restored_windows
                        .set_value(true, ctx)
                );
            });
        });

        let workspace = mock_workspace(&mut app);

        let closed_snapshot = workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = false;
            workspace.snapshot(ctx.window_id(), false, ctx)
        });

        let restored = restored_workspace(&mut app, closed_snapshot);
        restored.read(&app, |workspace, _| {
            assert!(workspace.vertical_tabs_panel_open);
        });
    });
}

#[test]
fn test_vertical_tabs_panel_closed_when_disabled_even_if_persisted_open() {
    // Regression for #9505: when `vertical_tabs_panel_open=true` is persisted
    // and the user then disables vertical tabs, restoring the workspace must
    // not honor the stale snapshot — otherwise a dismiss underlay paints over
    // the window and silently swallows every click.
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Snapshot the workspace with the panel open while vertical tabs are enabled.
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
        });
        let workspace = mock_workspace(&mut app);
        let open_snapshot = workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = true;
            workspace.snapshot(ctx.window_id(), false, ctx)
        });

        // Disable vertical tabs, then restore. The panel must stay closed.
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(false, ctx));
            });
        });
        let restored = restored_workspace(&mut app, open_snapshot);
        restored.read(&app, |workspace, _| {
            assert!(!workspace.vertical_tabs_panel_open);
        });
    });
}

#[test]
fn test_vertical_tabs_panel_defaults_open_for_new_window_when_vertical_tabs_enabled() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
        });

        let workspace = mock_workspace(&mut app);

        workspace.read(&app, |workspace, _| {
            assert!(workspace.vertical_tabs_panel_open);
        });
    });
}

#[test]
fn test_vertical_tabs_panel_inherits_transferred_tab_source_window_state() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
        });

        let transferred_closed = transferred_tab_workspace(&mut app, false);
        let transferred_open = transferred_tab_workspace(&mut app, true);

        transferred_closed.read(&app, |workspace, _| {
            assert!(!workspace.vertical_tabs_panel_open);
        });
        transferred_open.read(&app, |workspace, _| {
            assert!(workspace.vertical_tabs_panel_open);
        });
    });
}

#[test]
fn test_vertical_tabs_panel_auto_shows_when_setting_enabled() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.read(&app, |workspace, _| {
            assert!(!workspace.vertical_tabs_panel_open);
        });

        // Enabling vertical tabs should auto-open the panel.
        workspace.update(&mut app, |_, ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
        });
        workspace.read(&app, |workspace, _| {
            assert!(workspace.vertical_tabs_panel_open);
        });

        // Disabling vertical tabs should auto-close the panel.
        workspace.update(&mut app, |_, ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(false, ctx));
            });
        });
        workspace.read(&app, |workspace, _| {
            assert!(!workspace.vertical_tabs_panel_open);
        });
    });
}

#[test]
fn test_active_tab_bar_position_id_tracks_layout() {
    // Cross-window drag hit-testing (`tab_bar_rects_for_window`) targets only
    // the active tab presentation. Regression guard for the bug where the
    // inactive horizontal bar registered as a drop zone while vertical tabs
    // were enabled, lighting up a spurious placeholder over the top bar.
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Horizontal tabs (setting off): the horizontal bar is the drop zone.
        app.read(|ctx| {
            assert_eq!(active_tab_bar_position_id(ctx), TAB_BAR_POSITION_ID);
        });

        // Vertical tabs (setting on): only the vertical panel is the drop zone,
        // so the horizontal bar no longer registers as a cross-window target.
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
        });
        app.read(|ctx| {
            assert_eq!(
                active_tab_bar_position_id(ctx),
                VERTICAL_TABS_PANEL_POSITION_ID
            );
        });
    });
}

#[test]
fn test_toggle_tab_configs_menu_opens_vertical_tabs_panel_and_menu() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
            workspace.vertical_tabs_panel_open = true;
        });
        workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = false;
            workspace.show_new_session_dropdown_menu = None;

            workspace.handle_action(&WorkspaceAction::ToggleTabConfigsMenu, ctx);

            assert!(workspace.vertical_tabs_panel_open);
            assert!(workspace.show_new_session_dropdown_menu.is_some());
        });
    });
}

#[test]
fn test_toggle_tab_configs_menu_keyboard_shortcut_selects_top_item() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.show_new_session_dropdown_menu = None;

            workspace.handle_action(&WorkspaceAction::ToggleTabConfigsMenu, ctx);

            assert!(workspace.show_new_session_dropdown_menu.is_some());
            assert_eq!(
                workspace
                    .new_session_dropdown_menu
                    .read(ctx, |menu, _| menu.selected_index()),
                Some(0)
            );
        });
    });
}

#[test]
fn test_pointer_opened_tab_configs_menu_does_not_select_top_item() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.toggle_new_session_dropdown_menu(
                crate::workspace::action::NewSessionMenuAnchor::Pointer(Vector2F::zero()),
                ctx,
            );

            assert!(workspace.show_new_session_dropdown_menu.is_some());
            assert_eq!(
                workspace
                    .new_session_dropdown_menu
                    .read(ctx, |menu, _| menu.selected_index()),
                None
            );
        });
    });
}

#[test]
fn test_new_session_menu_is_capped_to_window_height() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let window_height = ctx
                .windows()
                .platform_window(ctx.window_id())
                .expect("expected workspace window")
                .size()
                .y();

            workspace.open_new_session_dropdown_menu(
                crate::workspace::action::NewSessionMenuAnchor::AddTabButton(Vector2F::zero()),
                ctx,
            );

            let expected_height =
                (window_height - NEW_SESSION_MENU_WINDOW_MARGIN - NEW_SESSION_MENU_CHROME_HEIGHT)
                    .max(NEW_SESSION_MENU_MIN_HEIGHT);
            let menu_height = workspace.new_session_menu_max_height(
                crate::workspace::action::NewSessionMenuAnchor::AddTabButton(Vector2F::zero()),
                ctx,
            );

            assert!((menu_height - expected_height).abs() < f32::EPSILON);

            workspace.open_new_session_dropdown_menu(
                crate::workspace::action::NewSessionMenuAnchor::AddTabButton(Vector2F::zero()),
                ctx,
            );
            assert!(workspace.show_new_session_dropdown_menu.is_some());
        });
    });
}

#[test]
fn test_open_tab_config_with_params_does_not_use_worktree_branch_as_implicit_title() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let tab_config = crate::tab_configs::TabConfig {
            name: "Untitled worktree".to_string(),
            title: None,
            color: None,
            panes: vec![TabConfigPaneNode {
                id: "main".to_string(),
                pane_type: Some(TabConfigPaneType::Terminal),
                split: None,
                children: None,
                is_focused: Some(true),
                directory: None,
                commands: Some(vec!["echo {{autogenerated_branch_name}}".to_string()]),
                shell: None,
            }],
            params: HashMap::new(),
            source_path: None,
        };

        workspace.update(&mut app, |workspace, ctx| {
            workspace.open_tab_config_with_params(
                tab_config.clone(),
                HashMap::new(),
                Some("mesa-coyote"),
                ctx,
            );
        });

        workspace.read(&app, |workspace, ctx| {
            assert_eq!(workspace.tab_count(), 2);
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .custom_title(ctx),
                None
            );
        });
    });
}

#[test]
fn test_open_tab_config_with_params_uses_explicit_title_template() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let tab_config = crate::tab_configs::TabConfig {
            name: "Titled worktree".to_string(),
            title: Some("{{autogenerated_branch_name}}".to_string()),
            color: None,
            panes: vec![TabConfigPaneNode {
                id: "main".to_string(),
                pane_type: Some(TabConfigPaneType::Terminal),
                split: None,
                children: None,
                is_focused: Some(true),
                directory: None,
                commands: Some(vec!["echo {{autogenerated_branch_name}}".to_string()]),
                shell: None,
            }],
            params: HashMap::new(),
            source_path: None,
        };

        workspace.update(&mut app, |workspace, ctx| {
            workspace.open_tab_config_with_params(
                tab_config.clone(),
                HashMap::new(),
                Some("mesa-coyote"),
                ctx,
            );
        });

        workspace.read(&app, |workspace, ctx| {
            assert_eq!(workspace.tab_count(), 2);
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .custom_title(ctx),
                Some("mesa-coyote".to_string())
            );
        });
    });
}
#[test]
fn test_toggle_tab_configs_menu_does_not_change_vertical_tabs_panel_in_horizontal_mode() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(false, ctx));
            });
            workspace.vertical_tabs_panel_open = true;
            workspace.show_new_session_dropdown_menu = None;

            workspace.handle_action(&WorkspaceAction::ToggleTabConfigsMenu, ctx);

            assert!(workspace.vertical_tabs_panel_open);
            assert!(workspace.show_new_session_dropdown_menu.is_some());
        });
    });
}

#[test]
fn test_unified_new_session_menu_uses_new_worktree_config_label_and_order() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let labels = workspace
                .unified_new_session_menu_items(ctx)
                .iter()
                .map(new_session_menu_label)
                .collect::<Vec<_>>();

            assert!(!labels.iter().any(|label| label == "Worktree in"));

            let separator_index = labels
                .iter()
                .position(|label| label == "---")
                .expect("expected a separator in the new-session menu");

            assert_eq!(
                labels.get(separator_index + 1),
                Some(&"New worktree config".to_string())
            );
            assert_eq!(
                labels.get(separator_index + 2),
                Some(&"New tab config".to_string())
            );
        });
    });
}

#[test]
fn test_unified_new_session_menu_includes_reopen_closed_session() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let menu_items = workspace.unified_new_session_menu_items(ctx);
            assert!(matches!(
                menu_items.get(menu_items.len() - 2),
                Some(MenuItem::Separator)
            ));

            let reopen_item = reopen_closed_session_menu_item(&menu_items);
            assert!(reopen_item.is_disabled());
            assert!(matches!(
                reopen_item.on_select_action(),
                Some(action) if matches!(action, WorkspaceAction::ReopenClosedSession)
            ));

            workspace.add_terminal_tab(false, ctx);
            workspace.remove_tab(workspace.active_tab_index(), true, true, ctx);

            let menu_items = workspace.unified_new_session_menu_items(ctx);
            let reopen_item = reopen_closed_session_menu_item(&menu_items);
            assert!(!reopen_item.is_disabled());
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_worktree_sidecar_search_editor_proxies_navigation_and_escape() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let temp_root = TempDir::new().expect("failed to create temp dir");
        let alpha_repo = temp_root.path().join("alpha-repo");
        let beta_repo = temp_root.path().join("beta-repo");
        std::fs::create_dir_all(&alpha_repo).expect("failed to create alpha repo dir");
        std::fs::create_dir_all(&beta_repo).expect("failed to create beta repo dir");

        workspace.update(&mut app, |_, ctx| {
            PersistedWorkspace::handle(ctx).update(ctx, |persisted, ctx| {
                persisted.user_added_workspace(alpha_repo.clone(), ctx);
                persisted.user_added_workspace(beta_repo.clone(), ctx);
            });
        });

        open_worktree_sidecar(&workspace, &mut app);

        workspace.read(&app, |workspace, ctx| {
            assert!(workspace.show_new_session_sidecar);
            assert!(workspace.worktree_sidecar_search_editor.is_focused(ctx));
            assert_eq!(
                workspace
                    .new_session_sidecar_menu
                    .read(ctx, |menu, _| menu.selected_index()),
                Some(1)
            );
        });

        workspace.update(&mut app, |workspace, ctx| {
            workspace
                .worktree_sidecar_search_editor
                .update(ctx, |_, ctx| {
                    ctx.emit(Event::Navigate(NavigationKey::Down));
                });
        });
        workspace.read(&app, |workspace, ctx| {
            assert_eq!(
                workspace
                    .new_session_sidecar_menu
                    .read(ctx, |menu, _| menu.selected_index()),
                Some(2)
            );
        });

        workspace.update(&mut app, |workspace, ctx| {
            workspace
                .worktree_sidecar_search_editor
                .update(ctx, |_, ctx| {
                    ctx.emit(Event::Navigate(NavigationKey::Up));
                });
        });
        workspace.read(&app, |workspace, ctx| {
            assert_eq!(
                workspace
                    .new_session_sidecar_menu
                    .read(ctx, |menu, _| menu.selected_index()),
                Some(1)
            );
        });

        workspace.update(&mut app, |workspace, ctx| {
            workspace
                .worktree_sidecar_search_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("beta", ctx);
                });
        });
        workspace.read(&app, |workspace, ctx| {
            assert_eq!(workspace.worktree_sidecar_search_query, "beta");
            assert_eq!(
                workspace
                    .new_session_sidecar_menu
                    .read(ctx, |menu, _| menu.items_len()),
                2
            );
            assert_eq!(
                workspace
                    .new_session_sidecar_menu
                    .read(ctx, |menu, _| menu.selected_index()),
                Some(1)
            );
        });

        workspace.update(&mut app, |workspace, ctx| {
            workspace
                .worktree_sidecar_search_editor
                .update(ctx, |_, ctx| {
                    ctx.emit(Event::Escape);
                });
        });
        workspace.read(&app, |workspace, ctx| {
            assert!(workspace.show_new_session_dropdown_menu.is_none());
            assert!(!workspace.show_new_session_sidecar);
            assert!(workspace.worktree_sidecar_search_query.is_empty());
            assert!(
                workspace
                    .worktree_sidecar_search_editor
                    .as_ref(ctx)
                    .buffer_text(ctx)
                    .is_empty()
            );
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_worktree_sidecar_hides_linked_worktrees_from_repo_list() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let temp_root = TempDir::new().expect("failed to create temp dir");
        let main_repo = temp_root.path().join("main-repo");
        let linked_worktree = temp_root.path().join("linked-worktree");
        let external_git_dir = main_repo
            .join(".git")
            .join("worktrees")
            .join("linked-worktree");

        std::fs::create_dir_all(&main_repo).expect("failed to create main repo dir");
        std::fs::create_dir_all(&linked_worktree).expect("failed to create linked worktree dir");
        std::fs::create_dir_all(&external_git_dir).expect("failed to create external git dir");

        workspace.update(&mut app, |_, ctx| {
            PersistedWorkspace::handle(ctx).update(ctx, |persisted, ctx| {
                persisted.user_added_workspace(main_repo.clone(), ctx);
                persisted.user_added_workspace(linked_worktree.clone(), ctx);
            });

            let main_repo_canon =
                CanonicalizedPath::try_from(main_repo.as_path()).expect("canonical main repo");
            let linked_worktree_canon = CanonicalizedPath::try_from(linked_worktree.as_path())
                .expect("canonical linked worktree");
            let external_git_dir_canon = CanonicalizedPath::try_from(external_git_dir.as_path())
                .expect("canonical external git dir");

            let main_repo_std: warp_util::standardized_path::StandardizedPath =
                main_repo_canon.into();
            let linked_worktree_std: warp_util::standardized_path::StandardizedPath =
                linked_worktree_canon.into();
            let external_git_dir_std: warp_util::standardized_path::StandardizedPath =
                external_git_dir_canon.into();

            DetectedRepositories::handle(ctx).update(ctx, |repos, _ctx| {
                repos.insert_test_repo_root(main_repo_std.clone());
                repos.insert_test_repo_root(linked_worktree_std.clone());
            });

            DirectoryWatcher::handle(ctx).update(ctx, |watcher, ctx| {
                watcher
                    .add_directory_with_git_dir(main_repo_std, None, ctx)
                    .expect("register main repo");
                watcher
                    .add_directory_with_git_dir(
                        linked_worktree_std,
                        Some(external_git_dir_std),
                        ctx,
                    )
                    .expect("register linked worktree");
            });
        });

        open_worktree_sidecar(&workspace, &mut app);

        workspace.read(&app, |workspace, ctx| {
            let labels = workspace.new_session_sidecar_menu.read(ctx, |menu, _| {
                menu.items()
                    .iter()
                    .filter_map(|item| match item {
                        MenuItem::Item(fields) => Some(fields.label().to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            });

            let main_repo_label = main_repo.to_string_lossy().to_string();
            let linked_worktree_label = linked_worktree.to_string_lossy().to_string();

            assert!(labels.iter().any(|label| label == "Search repos"));
            assert!(labels.iter().any(|label| label == &main_repo_label));
            assert!(!labels.iter().any(|label| label == &linked_worktree_label));
        });
    });
}

#[test]
fn test_vertical_tabs_context_menu_does_not_show_hover_only_tab_bar() {
    let _full_screen_zen_mode_guard = FeatureFlag::FullScreenZenMode.override_enabled(true);
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(
                    settings
                        .workspace_decoration_visibility
                        .set_value(WorkspaceDecorationVisibility::OnHover, ctx)
                );
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
            workspace.should_show_ai_assistant_warm_welcome = false;
            workspace.vertical_tabs_panel_open = true;

            workspace.show_tab_right_click_menu =
                Some((0, TabContextMenuAnchor::Pointer(Vector2F::zero())));

            assert_eq!(workspace.tab_bar_mode(ctx), ShowTabBar::Hidden);
        });
    });
}

#[test]
fn test_standard_tab_context_menu_shows_hover_only_tab_bar() {
    let _full_screen_zen_mode_guard = FeatureFlag::FullScreenZenMode.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(
                    settings
                        .workspace_decoration_visibility
                        .set_value(WorkspaceDecorationVisibility::OnHover, ctx)
                );
            });
            workspace.should_show_ai_assistant_warm_welcome = false;

            workspace.show_tab_right_click_menu =
                Some((0, TabContextMenuAnchor::Pointer(Vector2F::zero())));

            assert_eq!(workspace.tab_bar_mode(ctx), ShowTabBar::Stacked);
        });
    });
}

#[test]
fn test_open_cloud_agent_setup_guide_action_opens_management_view_and_is_idempotent() {
    let _agent_management_guard = FeatureFlag::AgentManagementView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            assert!(
                !workspace
                    .current_workspace_state
                    .is_agent_management_view_open
            );

            workspace.handle_action(&WorkspaceAction::OpenCloudAgentSetupGuide, ctx);
            assert!(
                workspace
                    .current_workspace_state
                    .is_agent_management_view_open
            );
            assert!(
                workspace
                    .agent_management_view
                    .as_ref(ctx)
                    .is_showing_setup_guide()
            );

            workspace.handle_action(&WorkspaceAction::OpenCloudAgentSetupGuide, ctx);
            assert!(
                workspace
                    .current_workspace_state
                    .is_agent_management_view_open
            );
            assert!(
                workspace
                    .agent_management_view
                    .as_ref(ctx)
                    .is_showing_setup_guide()
            );
        });
    });
}

#[test]
fn test_tab_mru_order() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);

            let id_a = workspace.tabs[0].pane_group.id();
            let id_b = workspace.tabs[1].pane_group.id();
            let id_c = workspace.tabs[2].pane_group.id();

            workspace.handle_action(&WorkspaceAction::ActivateTab(0), ctx);
            workspace.handle_action(&WorkspaceAction::ActivateTab(1), ctx);
            workspace.handle_action(&WorkspaceAction::ActivateTab(2), ctx);
            workspace.handle_action(&WorkspaceAction::ActivateTab(0), ctx);

            assert_eq!(workspace.tab_mru_order(), &[id_a, id_c, id_b]);
        });
    });
}

#[test]
fn test_create_new_tab_group_groups_active_tab() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // Workspace starts with one tab from `Empty` source. Create a tab
            // group and verify the active tab is assigned to it.
            assert_eq!(workspace.tab_count(), 1);
            assert!(workspace.tabs[0].group_id.is_none());
            assert!(workspace.tab_groups.is_empty());

            workspace.handle_action(
                &WorkspaceAction::SelectNewSessionMenuItem(NewSessionMenuItem::CreateNewTabGroup),
                ctx,
            );

            assert_eq!(workspace.tab_groups.len(), 1);
            let group_id = workspace.tabs[0]
                .group_id
                .expect("active tab should be assigned to the new group");
            assert!(workspace.tab_groups.contains_key(&group_id));
            // New groups start expanded so members are visible.
            assert!(!workspace.tab_groups[&group_id].collapsed);
        });
    });
}

#[test]
fn test_new_tab_group_from_tab_keeps_tab_in_place() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // Build a three-tab workspace (starts with one tab).
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            assert_eq!(workspace.tab_count(), 3);

            // Target the last tab so a move-to-top would be observable.
            let target_index = workspace.tab_count() - 1;
            let target_id = workspace.tabs[target_index].pane_group.id();

            workspace.handle_action(&WorkspaceAction::NewTabGroupFromTab(target_index), ctx);

            // The tab stays at its original index instead of jumping to the top.
            assert_eq!(workspace.tabs[target_index].pane_group.id(), target_id);
            assert_eq!(workspace.active_tab_index(), target_index);

            let group_id = workspace.tabs[target_index]
                .group_id
                .expect("target tab should be assigned to the new group");
            assert!(workspace.tab_groups.contains_key(&group_id));

            // The other tabs remain ungrouped.
            assert!(workspace.tabs[0].group_id.is_none());
            assert!(workspace.tabs[1].group_id.is_none());
        });
    });
}

#[test]
fn test_new_tab_group_from_selected_tabs_anchors_at_earliest_tab() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // Build a four-tab workspace (starts with one tab).
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            assert_eq!(workspace.tab_count(), 4);

            let id0 = workspace.tabs[0].pane_group.id();
            let id1 = workspace.tabs[1].pane_group.id();
            let id2 = workspace.tabs[2].pane_group.id();
            let id3 = workspace.tabs[3].pane_group.id();

            // Select two non-adjacent tabs (indices 1 and 3) with the active
            // tab among the selection so no extra tab is pulled in.
            workspace.activate_tab(1, ctx);
            workspace.tabs[1].in_multi_selection = true;
            workspace.tabs[3].in_multi_selection = true;

            workspace.handle_action(&WorkspaceAction::NewTabGroupFromSelectedTabs, ctx);

            // The group block lands at the earliest selected tab's position
            // (index 1), preserving the relative order of its members.
            let order: Vec<_> = workspace
                .tabs
                .iter()
                .map(|tab| tab.pane_group.id())
                .collect();
            assert_eq!(order, vec![id0, id1, id3, id2]);

            // Both selected tabs share the new group; the others stay ungrouped.
            let group_id = workspace.tabs[1]
                .group_id
                .expect("earliest selected tab should join the new group");
            assert_eq!(workspace.tabs[2].group_id, Some(group_id));
            assert!(workspace.tabs[0].group_id.is_none());
            assert!(workspace.tabs[3].group_id.is_none());

            // Selection flags are cleared after grouping.
            assert!(workspace.tabs.iter().all(|tab| !tab.in_multi_selection));
        });
    });
}

#[test]
fn test_new_tab_group_from_tab_in_group_anchors_after_group() {
    // Pulling a tab out of the middle of an existing group must not split
    // that group: the new single-tab group should land just past the old
    // group's last remaining member.
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // Build a four-tab workspace (starts with one tab).
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            assert_eq!(workspace.tab_count(), 4);

            // Group the first three tabs (indices 0, 1, 2) into one group,
            // leaving the last tab ungrouped.
            let group = TabGroup::new();
            let existing_group_id = group.id;
            workspace.tab_groups.insert(existing_group_id, group);
            for index in 0..3 {
                workspace.tabs[index].group_id = Some(existing_group_id);
            }

            let id0 = workspace.tabs[0].pane_group.id();
            let middle_id = workspace.tabs[1].pane_group.id();
            let id2 = workspace.tabs[2].pane_group.id();
            let id3 = workspace.tabs[3].pane_group.id();

            // Create a new group from the middle member of the existing group.
            workspace.handle_action(&WorkspaceAction::NewTabGroupFromTab(1), ctx);

            // The pulled tab lands right after the old group's run, so the old
            // group's members stay contiguous: [g0, g2, new, ungrouped].
            let order: Vec<_> = workspace
                .tabs
                .iter()
                .map(|tab| tab.pane_group.id())
                .collect();
            assert_eq!(order, vec![id0, id2, middle_id, id3]);

            let new_group_id = workspace.tabs[2]
                .group_id
                .expect("pulled tab should be in the new group");
            assert_ne!(new_group_id, existing_group_id);
            assert_eq!(workspace.active_tab_index(), 2);

            // The old group keeps its two contiguous members.
            assert_eq!(workspace.tabs[0].group_id, Some(existing_group_id));
            assert_eq!(workspace.tabs[1].group_id, Some(existing_group_id));
            assert!(workspace.tabs[3].group_id.is_none());
        });
    });
}

#[test]
fn test_new_tab_group_from_selected_tabs_in_group_anchors_after_group() {
    // When the earliest selected tab sits inside an existing group, the new
    // group block is anchored past that group's last surviving member so the
    // existing group is never split.
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // Build a five-tab workspace (starts with one tab).
            for _ in 0..4 {
                workspace.add_terminal_tab(false, ctx);
            }
            assert_eq!(workspace.tab_count(), 5);

            // Group the first three tabs (indices 0, 1, 2); leave 3 and 4 loose.
            let group = TabGroup::new();
            let existing_group_id = group.id;
            workspace.tab_groups.insert(existing_group_id, group);
            for index in 0..3 {
                workspace.tabs[index].group_id = Some(existing_group_id);
            }

            let id0 = workspace.tabs[0].pane_group.id();
            let id1 = workspace.tabs[1].pane_group.id();
            let id2 = workspace.tabs[2].pane_group.id();
            let id3 = workspace.tabs[3].pane_group.id();
            let id4 = workspace.tabs[4].pane_group.id();

            // Select the middle member of the group (index 1) and the trailing
            // ungrouped tab (index 4), with the active tab among the selection.
            workspace.activate_tab(1, ctx);
            workspace.tabs[1].in_multi_selection = true;
            workspace.tabs[4].in_multi_selection = true;

            workspace.handle_action(&WorkspaceAction::NewTabGroupFromSelectedTabs, ctx);

            // The block is placed just past the old group's surviving run
            // (g0, g2), keeping that group contiguous:
            // [g0, g2, new1, new4, id3].
            let order: Vec<_> = workspace
                .tabs
                .iter()
                .map(|tab| tab.pane_group.id())
                .collect();
            assert_eq!(order, vec![id0, id2, id1, id4, id3]);

            let new_group_id = workspace.tabs[2]
                .group_id
                .expect("selected tab should be in the new group");
            assert_ne!(new_group_id, existing_group_id);
            assert_eq!(workspace.tabs[3].group_id, Some(new_group_id));

            // Old group stays contiguous with its two surviving members.
            assert_eq!(workspace.tabs[0].group_id, Some(existing_group_id));
            assert_eq!(workspace.tabs[1].group_id, Some(existing_group_id));
            assert!(workspace.tabs[4].group_id.is_none());
        });
    });
}

#[test]
fn test_toggle_tab_group_collapsed_flips_state() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(
                &WorkspaceAction::SelectNewSessionMenuItem(NewSessionMenuItem::CreateNewTabGroup),
                ctx,
            );
            let group_id = workspace.tabs[0]
                .group_id
                .expect("active tab should be in a group");
            assert!(!workspace.tab_groups[&group_id].collapsed);

            workspace.handle_action(&WorkspaceAction::ToggleTabGroupCollapsed(group_id), ctx);
            assert!(workspace.tab_groups[&group_id].collapsed);

            workspace.handle_action(&WorkspaceAction::ToggleTabGroupCollapsed(group_id), ctx);
            assert!(!workspace.tab_groups[&group_id].collapsed);
        });
    });
}

#[test]
fn test_close_tab_group_removes_group_and_members() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // Create a group, then add another tab which inherits the
            // active tab's group_id via `add_tab_with_pane_layout`.
            workspace.handle_action(
                &WorkspaceAction::SelectNewSessionMenuItem(NewSessionMenuItem::CreateNewTabGroup),
                ctx,
            );
            let group_id = workspace.tabs[workspace.active_tab_index()]
                .group_id
                .expect("active tab should be in a group");

            workspace.add_terminal_tab(false, ctx);

            let group_members: Vec<usize> = workspace
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, tab)| tab.group_id == Some(group_id))
                .map(|(idx, _)| idx)
                .collect();
            assert_eq!(
                group_members.len(),
                2,
                "new tab should inherit the active tab's group_id"
            );

            workspace.handle_action(&WorkspaceAction::CloseTabGroup(group_id), ctx);

            // All group members are closed and the group entry is removed.
            assert!(!workspace.tab_groups.contains_key(&group_id));
            assert!(
                workspace
                    .tabs
                    .iter()
                    .all(|tab| tab.group_id != Some(group_id))
            );
        });
    });
}

#[test]
fn test_new_tab_with_after_all_tabs_setting_lands_top_level_at_end() {
    // With `new_tab_placement = AfterAllTabs`, a new tab lands at the very end
    // of the tab bar, outside any group — even when the active tab is in a
    // group.
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(
                    settings
                        .new_tab_placement
                        .set_value(NewTabPlacement::AfterAllTabs, ctx)
                );
            });
        });

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // Build [g0, g1, ungrouped] by assigning membership directly, so the
            // setup doesn't depend on new-tab placement behavior.
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            assert_eq!(workspace.tab_count(), 3);

            let group = TabGroup::new();
            let group_id = group.id;
            workspace.tab_groups.insert(group_id, group);
            workspace.tabs[0].group_id = Some(group_id);
            workspace.tabs[1].group_id = Some(group_id);

            // Activate a member of the group, then add a new tab.
            workspace.activate_tab(0, ctx);
            workspace.add_terminal_tab(false, ctx);

            // The new tab lands at the very end of the bar and is top-level.
            let last = workspace.tab_count() - 1;
            assert_eq!(workspace.active_tab_index(), last);
            assert_eq!(workspace.tabs[last].group_id, None);

            // The group keeps exactly its original two contiguous members.
            let group_members: Vec<usize> = workspace
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| t.group_id == Some(group_id))
                .map(|(idx, _)| idx)
                .collect();
            assert_eq!(group_members, vec![0, 1]);
        });
    });
}

#[test]
fn test_new_tab_with_after_current_tab_setting_lands_after_active_tab_in_group() {
    // With `new_tab_placement = AfterCurrentTab` and the active tab in the
    // middle of a group, a new tab should land immediately after the active
    // tab and inherit the group_id, preserving group contiguity rather than
    // jumping to the end of the group or past it.
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(
                    settings
                        .new_tab_placement
                        .set_value(NewTabPlacement::AfterCurrentTab, ctx)
                );
            });
        });

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // Create a group and grow it to two contiguous members so we can
            // activate the first one (i.e. a member that isn't at the end of
            // the group's run).
            workspace.handle_action(
                &WorkspaceAction::SelectNewSessionMenuItem(NewSessionMenuItem::CreateNewTabGroup),
                ctx,
            );
            let group_id = workspace.tabs[workspace.active_tab_index()]
                .group_id
                .expect("active tab should be in a group");
            workspace.add_terminal_tab(false, ctx);

            // Activate the first grouped tab so the next insertion happens in
            // the middle of the group's contiguous run.
            let first_grouped_idx = workspace
                .tabs
                .iter()
                .position(|t| t.group_id == Some(group_id))
                .expect("expected at least one grouped tab");
            workspace.activate_tab(first_grouped_idx, ctx);

            let expected_new_idx = first_grouped_idx + 1;

            workspace.add_terminal_tab(false, ctx);

            // The new tab lands immediately after the previously-active
            // grouped tab, inherits its group_id, and keeps the group's run
            // contiguous.
            assert_eq!(workspace.active_tab_index(), expected_new_idx);
            assert_eq!(
                workspace.tabs[expected_new_idx].group_id,
                Some(group_id),
                "new tab should inherit the active tab's group_id"
            );

            let group_indices: Vec<usize> = workspace
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| t.group_id == Some(group_id))
                .map(|(idx, _)| idx)
                .collect();
            assert_eq!(
                group_indices.len(),
                3,
                "group should have grown to three members"
            );
            assert!(
                group_indices.windows(2).all(|w| w[1] == w[0] + 1),
                "group's tab indices should be contiguous, got {group_indices:?}"
            );
        });
    });
}

#[test]
fn test_move_tab_to_group_expands_collapsed_group() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // Create a group and a second ungrouped tab.
            workspace.handle_action(
                &WorkspaceAction::SelectNewSessionMenuItem(NewSessionMenuItem::CreateNewTabGroup),
                ctx,
            );
            let group_id = workspace.tabs[workspace.active_tab_index()]
                .group_id
                .expect("active tab should be in a group");
            workspace.add_terminal_tab(false, ctx);

            // Find the ungrouped tab.
            let ungrouped_idx = workspace
                .tabs
                .iter()
                .position(|t| t.group_id.is_none())
                .expect("expected an ungrouped tab");

            // Collapse the group, then move the ungrouped tab into it.
            workspace.handle_action(&WorkspaceAction::ToggleTabGroupCollapsed(group_id), ctx);
            assert!(
                workspace.tab_groups[&group_id].collapsed,
                "group should be collapsed"
            );

            workspace.handle_action(
                &WorkspaceAction::MoveTabToGroup {
                    tab_index: ungrouped_idx,
                    group_id,
                },
                ctx,
            );

            assert!(
                !workspace.tab_groups[&group_id].collapsed,
                "group should expand when a tab is moved into it"
            );
        });
    });
}

#[test]
fn test_move_selected_tabs_to_group_expands_collapsed_group() {
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // Add two extra tabs while no group exists so they remain ungrouped.
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);

            // Create a group from the first tab (moves it to index 0) leaving
            // the other two tabs ungrouped.
            workspace.handle_action(&WorkspaceAction::NewTabGroupFromTab(0), ctx);
            let group_id = workspace.tabs[0]
                .group_id
                .expect("tab 0 should be in the new group");

            // Collapse the group.
            workspace.handle_action(&WorkspaceAction::ToggleTabGroupCollapsed(group_id), ctx);
            assert!(
                workspace.tab_groups[&group_id].collapsed,
                "group should be collapsed"
            );

            // Select the two ungrouped tabs and move them to the group.
            let ungrouped_indices: Vec<usize> = workspace
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| t.group_id.is_none())
                .map(|(i, _)| i)
                .collect();
            assert_eq!(ungrouped_indices.len(), 2);
            workspace.activate_tab(ungrouped_indices[0], ctx);
            workspace.tabs[ungrouped_indices[0]].in_multi_selection = true;
            workspace.tabs[ungrouped_indices[1]].in_multi_selection = true;

            workspace.handle_action(&WorkspaceAction::MoveSelectedTabsToGroup { group_id }, ctx);

            assert!(
                !workspace.tab_groups[&group_id].collapsed,
                "group should expand when selected tabs are moved into it"
            );
        });
    });
}

#[test]
fn test_new_tab_in_group_expands_collapsed_group_non_member_active() {
    // When the active tab is NOT a member of the group, `new_tab_in_group`
    // must still expand the target group.
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // Create a group, then activate an ungrouped tab so the active
            // tab is NOT a member of the group.
            workspace.handle_action(
                &WorkspaceAction::SelectNewSessionMenuItem(NewSessionMenuItem::CreateNewTabGroup),
                ctx,
            );
            let group_id = workspace.tabs[workspace.active_tab_index()]
                .group_id
                .expect("active tab should be in a group");
            workspace.add_terminal_tab(false, ctx);

            let ungrouped_idx = workspace
                .tabs
                .iter()
                .position(|t| t.group_id.is_none())
                .expect("expected an ungrouped tab");
            workspace.activate_tab(ungrouped_idx, ctx);

            // Collapse the group, then open a new tab inside it.
            workspace.handle_action(&WorkspaceAction::ToggleTabGroupCollapsed(group_id), ctx);
            assert!(
                workspace.tab_groups[&group_id].collapsed,
                "group should be collapsed"
            );

            workspace.handle_action(&WorkspaceAction::NewTabInGroup(group_id), ctx);

            assert!(
                !workspace.tab_groups[&group_id].collapsed,
                "group should expand when a new tab is opened in it"
            );
        });
    });
}

#[test]
fn test_new_tab_in_group_expands_collapsed_group_member_active() {
    // When the active tab IS a member of the group, `new_tab_in_group` takes
    // the inheritance path; the group must still expand.
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(
                &WorkspaceAction::SelectNewSessionMenuItem(NewSessionMenuItem::CreateNewTabGroup),
                ctx,
            );
            let group_id = workspace.tabs[workspace.active_tab_index()]
                .group_id
                .expect("active tab should be in a group");

            // Collapse the group, keeping the group member as the active tab.
            workspace.handle_action(&WorkspaceAction::ToggleTabGroupCollapsed(group_id), ctx);
            assert!(
                workspace.tab_groups[&group_id].collapsed,
                "group should be collapsed"
            );

            workspace.handle_action(&WorkspaceAction::NewTabInGroup(group_id), ctx);

            assert!(
                !workspace.tab_groups[&group_id].collapsed,
                "group should expand when a new tab is opened in it"
            );
        });
    });
}

#[test]
fn test_pin_unpin_ungrouped_tab_moves_to_and_from_boundary() {
    let _pinned_guard = FeatureFlag::PinnedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            assert_eq!(workspace.tab_count(), 3);

            let id0 = workspace.tabs[0].pane_group.id();
            let id1 = workspace.tabs[1].pane_group.id();
            let id2 = workspace.tabs[2].pane_group.id();

            // Pin tab at index 2: it should move to the front of the list.
            workspace.handle_action(&WorkspaceAction::PinTab(2), ctx);
            let order: Vec<_> = workspace.tabs.iter().map(|t| t.pane_group.id()).collect();
            assert_eq!(order, vec![id2, id0, id1]);
            assert!(workspace.tabs[0].pinned);
            assert!(!workspace.tabs[1].pinned);
            assert!(!workspace.tabs[2].pinned);

            // Unpin tab at index 0: it should move to the start of the unpinned region.
            workspace.handle_action(&WorkspaceAction::UnpinTab(0), ctx);
            let order: Vec<_> = workspace.tabs.iter().map(|t| t.pane_group.id()).collect();
            assert_eq!(order, vec![id2, id0, id1]);
            assert!(workspace.tabs.iter().all(|t| !t.pinned));
        });
    });
}

#[test]
fn test_pin_unpin_tab_group_moves_block_without_syncing_members() {
    // The group's own `pinned` flag is the sole source of truth for grouped
    // tabs — members keep `tab.pinned = false` regardless.
    let _pinned_guard = FeatureFlag::PinnedTabs.override_enabled(true);
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            assert_eq!(workspace.tab_count(), 4);

            let id0 = workspace.tabs[0].pane_group.id();
            let id1 = workspace.tabs[1].pane_group.id();
            let id2 = workspace.tabs[2].pane_group.id();
            let id3 = workspace.tabs[3].pane_group.id();

            // Group tabs at indices 2, 3.
            let group = TabGroup::new();
            let group_id = group.id;
            workspace.tab_groups.insert(group_id, group);
            workspace.tabs[2].group_id = Some(group_id);
            workspace.tabs[3].group_id = Some(group_id);

            // Pin the group: the block moves to the front; only the group's
            // flag flips — member tabs keep `pinned = false`.
            workspace.handle_action(&WorkspaceAction::PinTabGroup(group_id), ctx);
            let order: Vec<_> = workspace.tabs.iter().map(|t| t.pane_group.id()).collect();
            assert_eq!(order, vec![id2, id3, id0, id1]);
            assert!(workspace.tab_groups[&group_id].pinned);
            assert!(workspace.tabs.iter().all(|t| !t.pinned));

            // Unpin the group: block moves to the start of the unpinned
            // region; group's flag clears.
            workspace.handle_action(&WorkspaceAction::UnpinTabGroup(group_id), ctx);
            assert!(!workspace.tab_groups[&group_id].pinned);
            assert!(workspace.tabs.iter().all(|t| !t.pinned));

            // Group is still contiguous.
            let group_indices: Vec<usize> = workspace
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| t.group_id == Some(group_id))
                .map(|(i, _)| i)
                .collect();
            assert_eq!(group_indices.len(), 2);
            assert_eq!(group_indices[1] - group_indices[0], 1);
        });
    });
}

#[test]
fn test_pin_tab_on_grouped_tab_extracts_then_pins() {
    let _pinned_guard = FeatureFlag::PinnedTabs.override_enabled(true);
    let _grouped_tabs_guard = FeatureFlag::GroupedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            assert_eq!(workspace.tab_count(), 3);

            let id0 = workspace.tabs[0].pane_group.id();
            let id1 = workspace.tabs[1].pane_group.id();
            let id2 = workspace.tabs[2].pane_group.id();

            // Group tabs 0 and 1; tab 1 is the target.
            let group = TabGroup::new();
            let group_id = group.id;
            workspace.tab_groups.insert(group_id, group);
            workspace.tabs[0].group_id = Some(group_id);
            workspace.tabs[1].group_id = Some(group_id);

            // Pin tab at index 1: extracts from group, then pins as ungrouped.
            workspace.handle_action(&WorkspaceAction::PinTab(1), ctx);

            // Pinned tab (id1) is at the front, ungrouped.
            assert_eq!(workspace.tabs[0].pane_group.id(), id1);
            assert!(workspace.tabs[0].pinned);
            assert!(workspace.tabs[0].group_id.is_none());

            // Source group still has its one remaining member (id0).
            assert_eq!(workspace.tabs[1].pane_group.id(), id0);
            assert_eq!(workspace.tabs[1].group_id, Some(group_id));
            assert!(!workspace.tabs[1].pinned);

            // Ungrouped tab id2 remains untouched.
            assert_eq!(workspace.tabs[2].pane_group.id(), id2);
            assert!(workspace.tabs[2].group_id.is_none());
            assert!(!workspace.tabs[2].pinned);
        });
    });
}

/// Regression for the tools-panel tab visibility toggles surfaced in the
/// Appearance settings page: toggling a tab's backing setting must add/remove
/// that tab from the tools panel live, and re-enabling Warp Drive must make it
/// selectable again (the original report was that Warp Drive could vanish from
/// the tools panel with no way back).
#[test]
fn test_tools_panel_warp_drive_toggle_updates_available_views() {
    // Force the non-anonymous path so `is_warp_drive_enabled` follows the
    // `enable_warp_drive` setting rather than the auth state.
    let _skip_anon_guard = FeatureFlag::SkipFirebaseAnonymousUser.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        // Warp Drive is enabled by default, so it is an available tools-panel
        // tab and can be made the active view.
        workspace.update(&mut app, |workspace, ctx| {
            assert!(
                workspace
                    .left_panel_views
                    .contains(&ToolPanelView::WarpDrive),
                "Warp Drive should be an available tools-panel tab by default"
            );
            workspace.left_panel_view.update(ctx, |lp, ctx| {
                lp.handle_action_with_force_open(&LeftPanelAction::WarpDrive, false, ctx);
            });
            assert_eq!(
                workspace.left_panel_view.as_ref(ctx).active_view(),
                ToolPanelView::WarpDrive,
                "Warp Drive should be selectable as the active view"
            );
        });

        // Turning the toggle off (via its backing setting) removes Warp Drive
        // from the tools panel; if other tabs remain the active view falls back
        // to one of them.
        app.update(|ctx| {
            WarpDriveSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .enable_warp_drive
                    .set_value(false, ctx)
                    .expect("disable warp drive");
            });
        });
        workspace.update(&mut app, |workspace, ctx| {
            assert!(
                !workspace
                    .left_panel_views
                    .contains(&ToolPanelView::WarpDrive),
                "Disabling the setting should remove Warp Drive from the tools panel"
            );
            if !workspace.left_panel_views.is_empty() {
                assert_ne!(
                    workspace.left_panel_view.as_ref(ctx).active_view(),
                    ToolPanelView::WarpDrive,
                    "Active view should fall back to a remaining tab when Warp Drive is removed"
                );
            }
        });

        // Re-enabling restores Warp Drive as a selectable tab.
        app.update(|ctx| {
            WarpDriveSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .enable_warp_drive
                    .set_value(true, ctx)
                    .expect("re-enable warp drive");
            });
        });
        workspace.update(&mut app, |workspace, ctx| {
            assert!(
                workspace
                    .left_panel_views
                    .contains(&ToolPanelView::WarpDrive),
                "Re-enabling the setting should restore Warp Drive to the tools panel"
            );
            workspace.left_panel_view.update(ctx, |lp, ctx| {
                lp.handle_action_with_force_open(&LeftPanelAction::WarpDrive, false, ctx);
            });
            assert_eq!(
                workspace.left_panel_view.as_ref(ctx).active_view(),
                ToolPanelView::WarpDrive,
                "Warp Drive should be selectable again after re-enabling"
            );
        });
    });
}
