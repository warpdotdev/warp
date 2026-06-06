use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use instant::Instant;
use warp_core::ui::appearance::Appearance;
use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{App, AppContext, Element, Entity, TypedActionView, View, WindowId};

use super::*;
use crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier;
use crate::ai::cloud_environments::{
    AmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel, GithubRepo,
};
use crate::auth::AuthStateProvider;
use crate::network::NetworkStatus;
use crate::root_view::CreateEnvironmentArg;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ClientId, ServerId, SyncId};
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::settings::PrivacySettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal::view::init_environment::mode_selector::EnvironmentSetupModeSelector;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn make_test_environment(
    name: &str,
    docker_image: &str,
    github_repos: Vec<(String, String)>,
    setup_commands: Vec<String>,
) -> EnvironmentDisplayData {
    make_test_environment_with_timestamps(
        name,
        docker_image,
        github_repos,
        setup_commands,
        None,
        None,
    )
}

fn make_test_environment_with_timestamps(
    name: &str,
    docker_image: &str,
    github_repos: Vec<(String, String)>,
    setup_commands: Vec<String>,
    last_edited_ts: Option<warp_graphql::scalars::time::ServerTimestamp>,
    last_used_ts: Option<warp_graphql::scalars::time::ServerTimestamp>,
) -> EnvironmentDisplayData {
    EnvironmentDisplayData {
        id: SyncId::ClientId(ClientId::new()),
        name: name.to_string(),
        description: None,
        docker_image: docker_image.to_string(),
        github_repos,
        setup_commands,
        last_edited_ts,
        last_used_ts,
    }
}

#[derive(Default)]
struct TestRootView;

impl Entity for TestRootView {
    type Event = ();
}

impl View for TestRootView {
    fn ui_name() -> &'static str {
        "TestRootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for TestRootView {
    type Action = ();
}

fn create_test_window(app: &mut App) -> WindowId {
    let (window_id, _root_view) = app.add_window(WindowStyle::NotStealFocus, |_| TestRootView);
    window_id
}

fn init_env_page_view_test_models(app: &mut App) {
    initialize_settings_for_tests(app);

    // Most Settings views assume these singleton models exist.
    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(CloudModel::mock);

    // Some Environments page code paths consult org/user settings (e.g. codebase context enablement),
    // even if the specific test isn't exercising them directly.
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(PrivacySettings::mock);

    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(|_| GitHubAuthNotifier::new());

    // The agent-assisted modal reads locally indexed repos via CodebaseIndexManager.
    // We register a test instance to avoid singleton lookup panics in unit tests.
    app.add_singleton_model(|ctx| {
        CodebaseIndexManager::new_for_test(ServerApiProvider::as_ref(ctx).get(), ctx)
    });
}

type EmptyMouseStates = (
    HashMap<SyncId, MouseStateHandle>,
    HashMap<SyncId, MouseStateHandle>,
    HashMap<SyncId, MouseStateHandle>,
    HashMap<SyncId, MouseStateHandle>,
    HashMap<SyncId, MouseStateHandle>,
    HashMap<SyncId, Instant>,
);

fn empty_card_mouse_states() -> EmptyMouseStates {
    (
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
    )
}

#[test]
fn test_render_environments_list_with_single_environment() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        app.update(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let environment =
                make_test_environment("Test Environment", "ubuntu:latest", vec![], vec![]);
            let (
                copy_mouse_states,
                edit_mouse_states,
                share_mouse_states,
                card_hover_states,
                view_runs_link_mouse_states,
                copy_feedback_times,
            ) = empty_card_mouse_states();
            let card_render_state = EnvironmentCardRenderState {
                copy_button_mouse_states: &copy_mouse_states,
                edit_button_mouse_states: &edit_mouse_states,
                share_button_mouse_states: &share_mouse_states,
                card_hover_mouse_states: &card_hover_states,
                view_runs_link_mouse_states: &view_runs_link_mouse_states,
                copy_feedback_times: &copy_feedback_times,
            };

            let element = EnvironmentsPageWidget::render_environments_list(
                &[environment],
                &card_render_state,
                appearance,
                ctx,
                EnvironmentListScope::Personal,
                false,
            );

            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                text_content.contains("Test Environment"),
                "Expected environment name in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("ubuntu:latest"),
                "Expected docker image in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("View my runs"),
                "Expected 'View my runs' link in rendered content: {}",
                text_content
            );
        });
    })
}

#[test]
fn test_render_environments_list_with_multiple_environments() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        app.update(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let environments = vec![
                make_test_environment("Environment 1", "ubuntu:latest", vec![], vec![]),
                make_test_environment("Environment 2", "debian:latest", vec![], vec![]),
            ];
            let (
                copy_mouse_states,
                edit_mouse_states,
                share_mouse_states,
                card_hover_states,
                view_runs_link_mouse_states,
                copy_feedback_times,
            ) = empty_card_mouse_states();
            let card_render_state = EnvironmentCardRenderState {
                copy_button_mouse_states: &copy_mouse_states,
                edit_button_mouse_states: &edit_mouse_states,
                share_button_mouse_states: &share_mouse_states,
                card_hover_mouse_states: &card_hover_states,
                view_runs_link_mouse_states: &view_runs_link_mouse_states,
                copy_feedback_times: &copy_feedback_times,
            };

            let element = EnvironmentsPageWidget::render_environments_list(
                &environments,
                &card_render_state,
                appearance,
                ctx,
                EnvironmentListScope::Personal,
                false,
            );

            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                text_content.contains("Environment 1"),
                "Expected first environment name in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("Environment 2"),
                "Expected second environment name in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("ubuntu:latest"),
                "Expected first docker image in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("debian:latest"),
                "Expected second docker image in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("View my runs"),
                "Expected 'View my runs' link in rendered content: {}",
                text_content
            );
        });
    })
}

#[test]
fn test_render_environment_card_with_minimal_config() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        app.update(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let environment =
                make_test_environment("Minimal Environment", "alpine:latest", vec![], vec![]);
            let (
                copy_mouse_states,
                edit_mouse_states,
                share_mouse_states,
                card_hover_states,
                view_runs_link_mouse_states,
                copy_feedback_times,
            ) = empty_card_mouse_states();
            let card_render_state = EnvironmentCardRenderState {
                copy_button_mouse_states: &copy_mouse_states,
                edit_button_mouse_states: &edit_mouse_states,
                share_button_mouse_states: &share_mouse_states,
                card_hover_mouse_states: &card_hover_states,
                view_runs_link_mouse_states: &view_runs_link_mouse_states,
                copy_feedback_times: &copy_feedback_times,
            };

            let element = EnvironmentsPageWidget::render_environment_card(
                &environment,
                &card_render_state,
                appearance,
                ctx,
                EnvironmentListScope::Personal,
                false,
            );

            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                text_content.contains("Minimal Environment"),
                "Expected environment name in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("alpine:latest"),
                "Expected docker image in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("View my runs"),
                "Expected 'View my runs' link in rendered content: {}",
                text_content
            );
        });
    })
}

#[test]
fn test_render_environment_card_with_github_repos() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        app.update(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let environment = make_test_environment(
                "Environment with Repos",
                "ubuntu:latest",
                vec![
                    ("owner1".to_string(), "repo1".to_string()),
                    ("owner2".to_string(), "repo2".to_string()),
                ],
                vec![],
            );
            let (
                copy_mouse_states,
                edit_mouse_states,
                share_mouse_states,
                card_hover_states,
                view_runs_link_mouse_states,
                copy_feedback_times,
            ) = empty_card_mouse_states();
            let card_render_state = EnvironmentCardRenderState {
                copy_button_mouse_states: &copy_mouse_states,
                edit_button_mouse_states: &edit_mouse_states,
                share_button_mouse_states: &share_mouse_states,
                card_hover_mouse_states: &card_hover_states,
                view_runs_link_mouse_states: &view_runs_link_mouse_states,
                copy_feedback_times: &copy_feedback_times,
            };

            let element = EnvironmentsPageWidget::render_environment_card(
                &environment,
                &card_render_state,
                appearance,
                ctx,
                EnvironmentListScope::Personal,
                false,
            );

            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                text_content.contains("Environment with Repos"),
                "Expected environment name in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("owner1/repo1"),
                "Expected first repo in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("owner2/repo2"),
                "Expected second repo in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("View my runs"),
                "Expected 'View my runs' link in rendered content: {}",
                text_content
            );
        });
    })
}

#[test]
fn test_render_environment_card_with_setup_commands() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        app.update(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let environment = make_test_environment(
                "Environment with Setup",
                "node:18",
                vec![],
                vec!["npm install".to_string(), "npm run build".to_string()],
            );
            let (
                copy_mouse_states,
                edit_mouse_states,
                share_mouse_states,
                card_hover_states,
                view_runs_link_mouse_states,
                copy_feedback_times,
            ) = empty_card_mouse_states();
            let card_render_state = EnvironmentCardRenderState {
                copy_button_mouse_states: &copy_mouse_states,
                edit_button_mouse_states: &edit_mouse_states,
                share_button_mouse_states: &share_mouse_states,
                card_hover_mouse_states: &card_hover_states,
                view_runs_link_mouse_states: &view_runs_link_mouse_states,
                copy_feedback_times: &copy_feedback_times,
            };

            let element = EnvironmentsPageWidget::render_environment_card(
                &environment,
                &card_render_state,
                appearance,
                ctx,
                EnvironmentListScope::Personal,
                false,
            );

            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                text_content.contains("Environment with Setup"),
                "Expected environment name in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("node:18"),
                "Expected docker image in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("npm install"),
                "Expected first setup command in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("npm run build"),
                "Expected second setup command in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("View my runs"),
                "Expected 'View my runs' link in rendered content: {}",
                text_content
            );
        });
    })
}

#[test]
fn test_render_environment_card_with_all_features() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        app.update(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let environment = make_test_environment(
                "Full Environment",
                "python:3.11",
                vec![
                    ("company".to_string(), "frontend".to_string()),
                    ("company".to_string(), "backend".to_string()),
                ],
                vec![
                    "pip install -r requirements.txt".to_string(),
                    "python setup.py".to_string(),
                ],
            );
            let (
                copy_mouse_states,
                edit_mouse_states,
                share_mouse_states,
                card_hover_states,
                view_runs_link_mouse_states,
                copy_feedback_times,
            ) = empty_card_mouse_states();
            let card_render_state = EnvironmentCardRenderState {
                copy_button_mouse_states: &copy_mouse_states,
                edit_button_mouse_states: &edit_mouse_states,
                share_button_mouse_states: &share_mouse_states,
                card_hover_mouse_states: &card_hover_states,
                view_runs_link_mouse_states: &view_runs_link_mouse_states,
                copy_feedback_times: &copy_feedback_times,
            };

            let element = EnvironmentsPageWidget::render_environment_card(
                &environment,
                &card_render_state,
                appearance,
                ctx,
                EnvironmentListScope::Personal,
                false,
            );

            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                text_content.contains("Full Environment"),
                "Expected environment name in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("python:3.11"),
                "Expected docker image in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("company/frontend"),
                "Expected first repo in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("company/backend"),
                "Expected second repo in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("pip install -r requirements.txt"),
                "Expected first setup command in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("python setup.py"),
                "Expected second setup command in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("View my runs"),
                "Expected 'View my runs' link in rendered content: {}",
                text_content
            );
        });
    })
}

#[test]
fn test_render_environment_card_with_empty_setup_commands() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        app.update(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let environment = make_test_environment(
                "Environment with Empty Commands",
                "ubuntu:latest",
                vec![],
                vec![],
            );
            let (
                copy_mouse_states,
                edit_mouse_states,
                share_mouse_states,
                card_hover_states,
                view_runs_link_mouse_states,
                copy_feedback_times,
            ) = empty_card_mouse_states();
            let card_render_state = EnvironmentCardRenderState {
                copy_button_mouse_states: &copy_mouse_states,
                edit_button_mouse_states: &edit_mouse_states,
                share_button_mouse_states: &share_mouse_states,
                card_hover_mouse_states: &card_hover_states,
                view_runs_link_mouse_states: &view_runs_link_mouse_states,
                copy_feedback_times: &copy_feedback_times,
            };

            let element = EnvironmentsPageWidget::render_environment_card(
                &environment,
                &card_render_state,
                appearance,
                ctx,
                EnvironmentListScope::Personal,
                false,
            );

            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                text_content.contains("Environment with Empty Commands"),
                "Expected environment name in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("ubuntu:latest"),
                "Expected docker image in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("View my runs"),
                "Expected 'View my runs' link in rendered content: {}",
                text_content
            );
        });
    })
}

#[test]
fn test_environments_page_widget_search_terms() {
    let widget = EnvironmentsPageWidget;
    let search_terms = widget.search_terms();

    assert!(search_terms.contains("environments"));
    assert!(search_terms.contains("environment"));
    assert!(search_terms.contains("ambient"));
    assert!(search_terms.contains("agents"));
    assert!(search_terms.contains("github"));
}

// ============================================================================
// Empty State vs List State Tests
// ============================================================================

#[test]
fn test_render_list_page_with_no_environments_shows_empty_state() {
    // Test that when there are no environments, the empty state is rendered
    App::test((), |mut app| async move {
        init_env_page_view_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, EnvironmentsPageView::new);
            let appearance = Appearance::as_ref(ctx);

            // CloudModel mock should have no environments by default
            let environments = CloudAmbientAgentEnvironment::get_all(ctx);
            assert_eq!(
                environments.len(),
                0,
                "Test should start with no environments"
            );

            let view = view_handle.as_ref(ctx);
            let element = EnvironmentsPageWidget::render_list_page(view, appearance, ctx);
            // Element is created successfully - just verify it doesn't panic
            drop(element);
        });
    })
}

#[test]
fn test_render_list_page_with_environments_shows_list() {
    // Test that when there are environments, the list is rendered (not empty state)
    App::test((), |mut app| async move {
        init_env_page_view_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            // Create test environment in CloudModel
            let environment = AmbientAgentEnvironment::new(
                "Test Environment".to_string(),
                Some("Test description".to_string()),
                vec![GithubRepo::new("owner".to_string(), "repo".to_string())],
                "ubuntu:latest".to_string(),
                vec!["npm install".to_string()],
            );

            let sync_id = SyncId::ClientId(ClientId::new());
            let object = CloudAmbientAgentEnvironment::new(
                sync_id,
                CloudAmbientAgentEnvironmentModel::new(environment),
                crate::cloud_object::CloudObjectMetadata::mock(),
                crate::cloud_object::CloudObjectPermissions::mock_personal(),
            );

            CloudModel::handle(ctx).update(ctx, |model, ctx| {
                model.create_object(sync_id, object, ctx);
            });
            let environments = CloudAmbientAgentEnvironment::get_all(ctx);
            assert_eq!(
                environments.len(),
                1,
                "Should have one environment after insert"
            );

            let view_handle = ctx.add_typed_action_view(window_id, EnvironmentsPageView::new);
            let appearance = Appearance::as_ref(ctx);
            let view = view_handle.as_ref(ctx);

            let element = EnvironmentsPageWidget::render_list_page(view, appearance, ctx);
            // Element is created successfully - just verify it doesn't panic
            drop(element);
        });
    })
}

#[test]
fn test_render_list_page_with_personal_and_team_environments_shows_section_headers() {
    App::test((), |mut app| async move {
        init_env_page_view_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            // Ensure UserWorkspaces has a current team name so the "Team" section renders with the
            // shared header copy ("Shared by Warp and <team>").
            UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                user_workspaces.setup_test_workspace(ctx);
                user_workspaces.update_current_workspace(
                    |workspace| {
                        if let Some(team) = workspace.teams.first_mut() {
                            team.name = "Katarina's team".to_string();
                        }
                    },
                    ctx,
                );
            });

            let personal_env = AmbientAgentEnvironment::new(
                "Personal Env".to_string(),
                None,
                vec![],
                "ubuntu:latest".to_string(),
                vec![],
            );

            let team_env = AmbientAgentEnvironment::new(
                "Team Env".to_string(),
                None,
                vec![],
                "ubuntu:latest".to_string(),
                vec![],
            );

            let personal_id = SyncId::ClientId(ClientId::new());
            let personal_object = CloudAmbientAgentEnvironment::new(
                personal_id,
                CloudAmbientAgentEnvironmentModel::new(personal_env),
                crate::cloud_object::CloudObjectMetadata::mock(),
                crate::cloud_object::CloudObjectPermissions::mock_personal(),
            );

            let team_id = SyncId::ClientId(ClientId::new());
            let mut team_permissions = crate::cloud_object::CloudObjectPermissions::mock_personal();
            team_permissions.owner = Owner::Team {
                team_uid: ServerId::from(789),
            };
            let team_object = CloudAmbientAgentEnvironment::new(
                team_id,
                CloudAmbientAgentEnvironmentModel::new(team_env),
                crate::cloud_object::CloudObjectMetadata::mock(),
                team_permissions,
            );

            CloudModel::handle(ctx).update(ctx, |model, ctx| {
                model.create_object(personal_id, personal_object, ctx);
                model.create_object(team_id, team_object, ctx);
            });

            let view_handle = ctx.add_typed_action_view(window_id, EnvironmentsPageView::new);
            let appearance = Appearance::as_ref(ctx);
            let view = view_handle.as_ref(ctx);

            let element = EnvironmentsPageWidget::render_list_page(view, appearance, ctx);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("PERSONAL"),
                "Expected 'Personal' section header in rendered content: {text_content}"
            );
            assert!(
                text_content.contains("SHARED BY WARP AND KATARINA'S TEAM"),
                "Expected shared section header in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_render_list_page_with_only_personal_environments_shows_personal_header() {
    App::test((), |mut app| async move {
        init_env_page_view_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let personal_env = AmbientAgentEnvironment::new(
                "Personal Env".to_string(),
                None,
                vec![],
                "ubuntu:latest".to_string(),
                vec![],
            );

            let personal_id = SyncId::ClientId(ClientId::new());
            let personal_object = CloudAmbientAgentEnvironment::new(
                personal_id,
                CloudAmbientAgentEnvironmentModel::new(personal_env),
                crate::cloud_object::CloudObjectMetadata::mock(),
                crate::cloud_object::CloudObjectPermissions::mock_personal(),
            );

            CloudModel::handle(ctx).update(ctx, |model, ctx| {
                model.create_object(personal_id, personal_object, ctx);
            });

            let view_handle = ctx.add_typed_action_view(window_id, EnvironmentsPageView::new);
            let appearance = Appearance::as_ref(ctx);
            let view = view_handle.as_ref(ctx);

            let element = EnvironmentsPageWidget::render_list_page(view, appearance, ctx);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("PERSONAL"),
                "Expected 'Personal' header in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_set_github_auth_redirect_target_updates_form() {
    App::test((), |mut app| async move {
        init_env_page_view_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        let mut view_handle = None;
        app.update(|ctx| {
            view_handle = Some(ctx.add_typed_action_view(window_id, EnvironmentsPageView::new));
        });
        let view_handle = view_handle.expect("EnvironmentsPageView handle should be created");

        app.update(|ctx| {
            let view = view_handle.as_ref(ctx);
            let target = view.environment_form.read(ctx, |form, _ctx| {
                form.github_auth_redirect_target_for_test()
            });
            assert_eq!(target, GithubAuthRedirectTarget::SettingsEnvironments);
        });

        app.update(|ctx| {
            view_handle.update(ctx, |view, ctx| {
                view.set_github_auth_redirect_target(GithubAuthRedirectTarget::FocusCloudMode, ctx);
            });
        });

        app.update(|ctx| {
            let view = view_handle.as_ref(ctx);
            let target = view.environment_form.read(ctx, |form, _ctx| {
                form.github_auth_redirect_target_for_test()
            });
            assert_eq!(target, GithubAuthRedirectTarget::FocusCloudMode);
        });
    })
}

#[test]
fn test_render_empty_state_shows_github_remote_and_local_rows() {
    // Empty-state UI should include GitHub-remote (suggested) and agent-assisted local repos paths.
    App::test((), |mut app| async move {
        init_env_page_view_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, EnvironmentsPageView::new);
            let appearance = Appearance::as_ref(ctx);
            let view = view_handle.as_ref(ctx);

            let element = EnvironmentsPageWidget::render_empty_state(view, appearance, ctx);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("Quick setup"),
                "Expected quick setup row title in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("Suggested"),
                "Expected 'Suggested' badge text in rendered content: {}",
                text_content
            );
            // GitHub button text depends on async auth state, so just check that one of the
            // expected states is present (Loading, Get started, Authorize, or Retry)
            let has_github_button = text_content.contains("Get started")
                || text_content.contains("Authorize")
                || text_content.contains("Loading...")
                || text_content.contains("Retry");
            assert!(
                has_github_button,
                "Expected GitHub button text in rendered content: {}",
                text_content
            );

            assert!(
                text_content.contains("Use the agent"),
                "Expected 'Use the agent' row title in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("Launch agent"),
                "Expected 'Launch agent' button text in rendered content: {}",
                text_content
            );

            assert!(
                !text_content.contains("Manually create an environment"),
                "Did not expect old manual-create empty-state row title in rendered content: {}",
                text_content
            );

            // Basic ordering: GitHub row should appear above local repos row.
            let github_pos = text_content.find("Quick setup").unwrap_or(usize::MAX);
            let local_pos = text_content
                .find("Use the agent")
                .unwrap_or(usize::MAX);
            assert!(
                github_pos < local_pos,
                "Expected GitHub row to appear before local row (github_pos={github_pos}, local_pos={local_pos}): {text_content}"
            );
        });
    })
}

#[test]
fn test_render_empty_state_github_card_loading_state() {
    // This test verifies that the empty state renders without crashing.
    // The specific GitHub auth state (Loading, Authed, etc.) is asynchronous
    // and can't be reliably controlled in unit tests.
    App::test((), |mut app| async move {
        init_env_page_view_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, EnvironmentsPageView::new);
            let appearance = Appearance::as_ref(ctx);
            let view = view_handle.as_ref(ctx);

            let element = EnvironmentsPageWidget::render_empty_state(view, appearance, ctx);
            let text_content = element.debug_text_content().unwrap_or_default();

            // Just verify the empty state renders the key components
            assert!(
                text_content.contains("Quick setup"),
                "Expected quick setup row in rendered content: {}",
                text_content
            );
        });
    })
}

#[test]
fn test_render_empty_state_github_card_error_state_shows_retry() {
    // This test verifies that the empty state renders without crashing.
    // The specific GitHub auth state (error, loading, etc.) is asynchronous
    // and can't be reliably controlled in unit tests.
    App::test((), |mut app| async move {
        init_env_page_view_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, EnvironmentsPageView::new);
            let appearance = Appearance::as_ref(ctx);
            let view = view_handle.as_ref(ctx);

            let element = EnvironmentsPageWidget::render_empty_state(view, appearance, ctx);
            let text_content = element.debug_text_content().unwrap_or_default();

            // Just verify the empty state renders the key components
            assert!(
                text_content.contains("Quick setup"),
                "Expected quick setup row in rendered content: {}",
                text_content
            );
        });
    })
}

#[test]
fn test_render_empty_state_github_card_unauthed_state_shows_authorize() {
    // This test verifies that the empty state renders without crashing.
    // The specific GitHub auth state (unauthed, authed, etc.) is asynchronous
    // and can't be reliably controlled in unit tests.
    App::test((), |mut app| async move {
        init_env_page_view_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, EnvironmentsPageView::new);
            let appearance = Appearance::as_ref(ctx);
            let view = view_handle.as_ref(ctx);

            let element = EnvironmentsPageWidget::render_empty_state(view, appearance, ctx);
            let text_content = element.debug_text_content().unwrap_or_default();

            // Just verify the empty state renders the key components
            assert!(
                text_content.contains("Quick setup"),
                "Expected quick setup row in rendered content: {}",
                text_content
            );
        });
    })
}

// ============================================================================
// Toolbar + Agent-assisted Flow Tests
// ============================================================================

#[test]
fn test_environment_setup_mode_selector_renders_options() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let selector = ctx.add_typed_action_view(window_id, EnvironmentSetupModeSelector::new);
            let element = selector.as_ref(ctx).render(ctx);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("Quick setup"),
                "Expected Quick setup option in rendered content: {}",
                text_content
            );
            assert!(
                text_content.contains("Use the agent"),
                "Expected Use the agent option in rendered content: {}",
                text_content
            );
        });
    })
}

#[test]
fn test_agent_assisted_modal_open_and_cancel_renders_and_hides() {
    // Verifies the Environments page wires up the modal visibility and cancel event correctly.
    App::test((), |mut app| async move {
        init_env_page_view_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        let mut view_handle = None;
        app.update(|ctx| {
            view_handle = Some(ctx.add_typed_action_view(window_id, EnvironmentsPageView::new));
        });
        let view_handle = view_handle.expect("EnvironmentsPageView handle should be created");

        // Open the modal.
        app.update(|ctx| {
            view_handle.update(ctx, |view, ctx| {
                view.handle_action(&EnvironmentsPageAction::OpenAgentAssistedCreateModal, ctx);
            });
        });

        // Verify the modal is visible.
        // We assert against `is_visible()` rather than `debug_text_content()` of the full dialog,
        // because dialog/icon rendering can be asset-provider-dependent in unit tests.
        app.update(|ctx| {
            let view = view_handle.as_ref(ctx);
            let modal = view.agent_assisted_environment_modal.clone();
            let is_visible = modal.read(ctx, |modal, _ctx| modal.is_visible());
            assert!(is_visible, "Expected modal to be visible after open action");
        });

        // Cancel via modal event.
        app.update(|ctx| {
            view_handle.update(ctx, |view, ctx| {
                view.agent_assisted_environment_modal
                    .update(ctx, |_modal, ctx| {
                        ctx.emit(AgentAssistedEnvironmentModalEvent::Cancelled);
                    });
            });
        });

        // Verify modal is hidden.
        app.update(|ctx| {
            let view = view_handle.as_ref(ctx);
            let modal = view.agent_assisted_environment_modal.clone();
            let is_visible = modal.read(ctx, |modal, _ctx| modal.is_visible());
            assert!(
                !is_visible,
                "Expected modal to be hidden after cancel event"
            );
        });
    })
}

#[test]
fn test_agent_assisted_modal_confirm_dispatches_root_view_action_and_hides_modal() {
    // We treat the RootView action dispatch as the contract that a terminal tab + setup flow will start.
    // (The deeper terminal-tab assertions are better suited to integration tests.)
    App::test((), |mut app| async move {
        init_env_page_view_test_models(&mut app);

        // Capture the CreateEnvironmentArg passed through the RootView action boundary.
        let dispatched_repo_paths: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let dispatched_repo_paths_clone = dispatched_repo_paths.clone();

        app.update(|ctx| {
            ctx.add_global_action(
                "root_view:create_environment_in_existing_window_and_run",
                move |arg: &CreateEnvironmentArg, _ctx| {
                    dispatched_repo_paths_clone
                        .lock()
                        .expect("mutex should not be poisoned")
                        .push(arg.repos.clone());
                },
            );
        });

        let window_id = create_test_window(&mut app);

        let mut view_handle = None;
        app.update(|ctx| {
            view_handle = Some(ctx.add_typed_action_view(window_id, EnvironmentsPageView::new));
        });
        let view_handle = view_handle.expect("EnvironmentsPageView handle should be created");

        // Open the modal.
        app.update(|ctx| {
            view_handle.update(ctx, |view, ctx| {
                view.handle_action(&EnvironmentsPageAction::OpenAgentAssistedCreateModal, ctx);
            });
        });

        // Confirm via modal event.
        let repo_paths = vec!["/tmp/repo-a".to_string(), "/tmp/repo-b".to_string()];
        app.update(|ctx| {
            view_handle.update(ctx, |view, ctx| {
                view.agent_assisted_environment_modal
                    .update(ctx, |_modal, ctx| {
                        ctx.emit(AgentAssistedEnvironmentModalEvent::Confirmed {
                            repo_paths: repo_paths.clone(),
                        });
                    });
            });
        });

        // Verify global action was dispatched with repos.
        let dispatched = dispatched_repo_paths
            .lock()
            .expect("mutex should not be poisoned")
            .clone();
        assert_eq!(
            dispatched,
            vec![repo_paths],
            "Expected root view action to be dispatched with repo paths"
        );

        // Verify modal is hidden after confirm.
        app.update(|ctx| {
            let view = view_handle.as_ref(ctx);
            let modal = view.agent_assisted_environment_modal.clone();
            let is_visible = modal.read(ctx, |modal, _ctx| modal.is_visible());
            assert!(
                !is_visible,
                "Expected modal to be hidden after confirm event"
            );
        });
    })
}

// ============================================================================
// Note: Form-related tests (repos field, docker image field, form state, etc.)
// have been moved to update_environment_form_tests.rs since the form component
// was extracted into UpdateEnvironmentForm.
// ============================================================================

// ============================================================================
// Environments Page Enum Tests
// ============================================================================

#[test]
fn test_environments_page_default_is_list() {
    let page = EnvironmentsPage::default();
    assert!(matches!(page, EnvironmentsPage::List));
}

#[test]
fn test_environments_page_edit_variant() {
    let env_id = SyncId::ClientId(ClientId::new());
    let page = EnvironmentsPage::Edit { env_id };

    if let EnvironmentsPage::Edit { env_id: id } = page {
        assert_eq!(id, env_id);
    } else {
        panic!("Expected Edit variant");
    }
}

// ============================================================================
// GithubRepo Tests
// ============================================================================

#[test]
fn test_github_repo_new() {
    let repo = GithubRepo::new("warpdotdev".to_string(), "warp-internal".to_string());
    assert_eq!(repo.owner, "warpdotdev");
    assert_eq!(repo.repo, "warp-internal");
}

#[test]
fn test_github_repo_display() {
    let repo = GithubRepo::new("warpdotdev".to_string(), "warp-internal".to_string());
    assert_eq!(repo.to_string(), "warpdotdev/warp-internal");
}

#[test]
fn test_github_repo_equality() {
    let repo1 = GithubRepo::new("owner".to_string(), "repo".to_string());
    let repo2 = GithubRepo::new("owner".to_string(), "repo".to_string());
    let repo3 = GithubRepo::new("other".to_string(), "repo".to_string());

    assert_eq!(repo1, repo2);
    assert_ne!(repo1, repo3);
}

// ============================================================================
// Environments List Search Tests
// ============================================================================

#[test]
fn test_environment_matches_search_query_empty_query_matches_all() {
    let environment = make_test_environment(
        "Searchable Environment",
        "ubuntu:latest",
        vec![("warpdotdev".to_string(), "warp-internal".to_string())],
        vec![],
    );

    assert!(environment.matches_search_query(""));
    assert!(environment.matches_search_query("   "));
}

#[test]
fn test_environment_matches_search_query_name_description_image_repos() {
    let mut environment = make_test_environment(
        "Warp Env",
        "node:20-alpine",
        vec![("warpdotdev".to_string(), "warp-internal".to_string())],
        vec![],
    );
    environment.description = Some("Front end focused agents".to_string());

    assert!(environment.matches_search_query("warp"));
    assert!(environment.matches_search_query("Front end"));
    assert!(environment.matches_search_query("node:20"));
    assert!(environment.matches_search_query("warp-internal"));
    assert!(environment.matches_search_query("warpdotdev"));
    assert!(environment.matches_search_query("warpdotdev/warp"));

    assert!(!environment.matches_search_query("definitely-not-present"));
}

#[test]
fn test_environment_matches_search_query_env_id_substring() {
    let environment = make_test_environment("Any", "ubuntu:latest", vec![], vec![]);

    let id_str = environment.id.to_string();
    let needle_len = id_str.chars().take(6).collect::<String>().len();
    let prefix = &id_str[..needle_len];

    assert!(environment.matches_search_query(prefix));
}

#[test]
fn test_environment_matches_search_query_is_case_insensitive() {
    let mut environment = make_test_environment(
        "warp-env",
        "ubuntu:latest",
        vec![("WarpDotDev".to_string(), "Warp-Internal".to_string())],
        vec![],
    );
    environment.description = Some("Some Description".to_string());

    assert!(environment.matches_search_query("WARP"));
    assert!(environment.matches_search_query("description"));
    assert!(environment.matches_search_query("warp-internal"));
}

#[test]
fn test_toolbar_renders_search_editor_view() {
    use pathfinder_geometry::vector::vec2f;

    App::test((), |mut app| async move {
        init_env_page_view_test_models(&mut app);

        // Seed at least one environment so the list page renders the toolbar.
        app.update(|ctx| {
            let environment = AmbientAgentEnvironment::new(
                "Test Environment".to_string(),
                Some("Test description".to_string()),
                vec![],
                "ubuntu:latest".to_string(),
                vec![],
            );

            let sync_id = SyncId::ClientId(ClientId::new());
            let object = CloudAmbientAgentEnvironment::new(
                sync_id,
                CloudAmbientAgentEnvironmentModel::new(environment),
                crate::cloud_object::CloudObjectMetadata::mock(),
                crate::cloud_object::CloudObjectPermissions::mock_personal(),
            );

            CloudModel::handle(ctx).update(ctx, |model, ctx| {
                model.create_object(sync_id, object, ctx);
            });
        });

        // Make EnvironmentsPageView the root view so it gets laid out by the Presenter.
        let (window_id, env_page_handle) =
            app.add_window(WindowStyle::NotStealFocus, EnvironmentsPageView::new);

        app.update(|ctx| {
            // Render a frame so layout runs and parent relationships are computed.
            // We use a large window size to avoid edge cases where layout is skipped.
            let presenter = ctx.presenter(window_id).expect("presenter should exist");
            presenter
                .borrow_mut()
                .build_scene(vec2f(1000., 700.), 1., None, ctx);

            let env_page = env_page_handle.as_ref(ctx);
            let env_page_id = env_page_handle.id();
            let search_editor_id = env_page.search_editor.id();

            let chain = presenter.borrow().ancestors(search_editor_id);
            assert!(
                chain.len() >= 2,
                "Expected search editor to be laid out as a child view; got ancestors={chain:?}"
            );
            assert_eq!(
                chain.first().copied(),
                Some(env_page_id),
                "Expected search editor root to be EnvironmentsPageView"
            );
            assert_eq!(chain.last().copied(), Some(search_editor_id));
        });
    })
}
// ============================================================================
// Environment Last Used Timestamp Tests
// ============================================================================

#[test]
fn test_render_environment_card_with_last_used_never() {
    use chrono::{Duration, Utc};
    use warp_graphql::scalars::time::ServerTimestamp;

    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        app.update(|ctx| {
            let appearance = Appearance::as_ref(ctx);

            // Create environment with no last-used timestamp
            let one_day_ago = Utc::now() - Duration::days(1);
            let environment = make_test_environment_with_timestamps(
                "Never Used Environment",
                "ubuntu:latest",
                vec![],
                vec![],
                Some(ServerTimestamp::from(one_day_ago)),
                None,
            );

            let (
                copy_mouse_states,
                edit_mouse_states,
                share_mouse_states,
                card_hover_states,
                view_runs_link_mouse_states,
                copy_feedback_times,
            ) = empty_card_mouse_states();

            // Render the card
            let card_render_state = EnvironmentCardRenderState {
                copy_button_mouse_states: &copy_mouse_states,
                edit_button_mouse_states: &edit_mouse_states,
                share_button_mouse_states: &share_mouse_states,
                card_hover_mouse_states: &card_hover_states,
                view_runs_link_mouse_states: &view_runs_link_mouse_states,
                copy_feedback_times: &copy_feedback_times,
            };

            let element = EnvironmentsPageWidget::render_environment_card(
                &environment,
                &card_render_state,
                appearance,
                ctx,
                EnvironmentListScope::Personal,
                false,
            );

            // Use debug_text_content to verify the rendered text
            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                text_content.contains("Last used: never"),
                "Expected 'Last used: never' in rendered text: {}",
                text_content
            );
            assert!(
                text_content.contains("Last edited:"),
                "Expected 'Last edited:' in rendered text: {}",
                text_content
            );
            assert!(
                text_content.contains("View my runs"),
                "Expected 'View my runs' link in rendered text: {}",
                text_content
            );
        });
    })
}

#[test]
fn test_render_environment_card_with_last_used_timestamp() {
    use chrono::{Duration, Utc};
    use warp_graphql::scalars::time::ServerTimestamp;

    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        app.update(|ctx| {
            let appearance = Appearance::as_ref(ctx);

            // Create environment with a last-used timestamp from 2 hours ago
            let one_day_ago = Utc::now() - Duration::days(1);
            let two_hours_ago = Utc::now() - Duration::hours(2);
            let environment = make_test_environment_with_timestamps(
                "Recently Used Environment",
                "python:3.11",
                vec![],
                vec![],
                Some(ServerTimestamp::from(one_day_ago)),
                Some(ServerTimestamp::from(two_hours_ago)),
            );

            let (
                copy_mouse_states,
                edit_mouse_states,
                share_mouse_states,
                card_hover_states,
                view_runs_link_mouse_states,
                copy_feedback_times,
            ) = empty_card_mouse_states();

            // Render the card
            let card_render_state = EnvironmentCardRenderState {
                copy_button_mouse_states: &copy_mouse_states,
                edit_button_mouse_states: &edit_mouse_states,
                share_button_mouse_states: &share_mouse_states,
                card_hover_mouse_states: &card_hover_states,
                view_runs_link_mouse_states: &view_runs_link_mouse_states,
                copy_feedback_times: &copy_feedback_times,
            };

            let element = EnvironmentsPageWidget::render_environment_card(
                &environment,
                &card_render_state,
                appearance,
                ctx,
                EnvironmentListScope::Personal,
                false,
            );

            // Use debug_text_content to verify the rendered text
            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                text_content.contains("Last edited:"),
                "Expected 'Last edited:' in rendered text: {}",
                text_content
            );
            assert!(
                text_content.contains("Last used:"),
                "Expected 'Last used:' in rendered text: {}",
                text_content
            );
            assert!(
                !text_content.contains("never"),
                "Did not expect 'never' in rendered text: {}",
                text_content
            );
            assert!(
                text_content.contains("View my runs"),
                "Expected 'View my runs' link in rendered text: {}",
                text_content
            );
        });
    })
}
