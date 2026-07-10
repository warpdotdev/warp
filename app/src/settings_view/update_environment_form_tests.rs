use url::Url;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{Empty, MouseStateHandle};
use warpui::platform::WindowStyle;
use warpui::{
    AddSingletonModel, App, AppContext, Element, Entity, SingletonEntity, TypedActionView, View,
    WindowId,
};

use super::{
    EnvironmentFormCopy, EnvironmentFormInitArgs, EnvironmentFormValues, SuggestImageState,
    UpdateEnvironmentForm, UpdateEnvironmentFormAction,
};
use crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier;
use crate::ai::ambient_agents::github_auth_url::{self, AuthSource, GithubAuthRedirectTarget};
use crate::ai::cloud_environments::{BaseImage, GithubRepo};
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ClientId, SyncId};
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::settings::PrivacySettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team::Team;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;

#[test]
fn test_parse_repo_input_owner_repo() {
    let (owner, repo) = UpdateEnvironmentForm::parse_repo_input("owner/repo")
        .expect("expected owner/repo to parse");
    assert_eq!(owner, "owner");
    assert_eq!(repo, "repo");
}

#[test]
fn test_parse_repo_input_github_url() {
    let (owner, repo) = UpdateEnvironmentForm::parse_repo_input("https://github.com/warp/warp.git")
        .expect("expected github url to parse");
    assert_eq!(owner, "warp");
    assert_eq!(repo, "warp");
}

#[test]
fn test_parse_repo_inputs_multiple_entries() {
    let parsed = UpdateEnvironmentForm::parse_repo_inputs(
        "https://github.com/warp/warp, warp/warp-internal\n git@github.com:warp/warp-server",
    );
    assert_eq!(
        parsed,
        vec![
            ("warp".to_string(), "warp".to_string()),
            ("warp".to_string(), "warp-internal".to_string()),
            ("warp".to_string(), "warp-server".to_string()),
        ]
    );
}

#[test]
fn test_parse_repo_inputs_invalid_returns_empty() {
    assert!(UpdateEnvironmentForm::parse_repo_inputs("not a repo").is_empty());
    assert!(UpdateEnvironmentForm::parse_repo_inputs("owner/").is_empty());
    assert!(UpdateEnvironmentForm::parse_repo_inputs("/repo").is_empty());
}

#[test]
fn test_build_auth_url_with_next_overrides_existing() {
    let base_url =
        "https://example.com/oauth/connect/github?foo=bar&next=old://settings/environments";
    let result = UpdateEnvironmentForm::build_auth_url_with_next(
        base_url,
        GithubAuthRedirectTarget::SettingsEnvironments,
        "warpdev",
    );
    let parsed = Url::parse(&result).expect("result should be valid url");
    let mut next_values = parsed
        .query_pairs()
        .filter(|(key, _)| key == "next")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    assert_eq!(next_values.len(), 1);
    assert_eq!(
        next_values.pop(),
        Some("warpdev://settings/environments".to_string())
    );
    assert!(parsed
        .query_pairs()
        .any(|(key, value)| key == "foo" && value == "bar"));
}

#[test]
fn test_build_auth_url_with_next_focus_cloud_mode() {
    let base_url = "https://example.com/oauth/connect/github";
    let result = UpdateEnvironmentForm::build_auth_url_with_next(
        base_url,
        GithubAuthRedirectTarget::FocusCloudMode,
        "warplocal",
    );
    let parsed = Url::parse(&result).expect("result should be valid url");
    let next_value = parsed
        .query_pairs()
        .find(|(key, _)| key == "next")
        .map(|(_, value)| value.into_owned());
    assert_eq!(
        next_value,
        Some("warplocal://action/focus_cloud_mode".to_string())
    );
}

#[test]
fn test_build_auth_url_with_next_cloud_setup_source() {
    let base_url = "https://example.com/oauth/connect/github";
    let result = github_auth_url::build_auth_url_with_next(
        base_url,
        GithubAuthRedirectTarget::FocusCloudMode,
        "warpdev",
        AuthSource::CloudSetup,
    );
    let parsed = Url::parse(&result).expect("result should be valid url");
    let next_value = parsed
        .query_pairs()
        .find(|(key, _)| key == "next")
        .map(|(_, value)| value.into_owned());
    assert_eq!(
        next_value,
        Some("warpdev://action/focus_cloud_mode?source=cloud_setup".to_string())
    );
}
#[test]
fn test_build_auth_url_with_next_uses_scheme_param() {
    let base_url = "https://example.com/oauth/connect/github?scheme=warp";
    let result = UpdateEnvironmentForm::build_auth_url_with_next(
        base_url,
        GithubAuthRedirectTarget::FocusCloudMode,
        "warplocal",
    );
    let parsed = Url::parse(&result).expect("result should be valid url");
    let next_value = parsed
        .query_pairs()
        .find(|(key, _)| key == "next")
        .map(|(_, value)| value.into_owned());
    assert_eq!(
        next_value,
        Some("warp://action/focus_cloud_mode".to_string())
    );
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

fn init_update_environment_form_test_models(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(|_| NetworkStatus::new());

    // These are required by some settings/UI code paths and by UserWorkspaces::update_workspaces.
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());

    app.add_singleton_model(|_| GitHubAuthNotifier::new());
}

#[derive(Debug)]
enum GithubAuthCallState {
    Loading,
    Authed,
    Unauthed { auth_url: String },
    Error { message: String },
}

impl GithubAuthCallState {
    fn unauthed(auth_url: impl Into<String>) -> Self {
        Self::Unauthed {
            auth_url: auth_url.into(),
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

fn set_github_auth_call_state(form: &mut UpdateEnvironmentForm, state: GithubAuthCallState) {
    match state {
        GithubAuthCallState::Loading => {
            form.github_dropdown_state.is_loading = true;
            form.github_dropdown_state.auth_url = None;
            form.github_dropdown_state.load_error_message = None;
        }
        GithubAuthCallState::Authed => {
            form.github_dropdown_state.is_loading = false;
            form.github_dropdown_state.auth_url = None;
            form.github_dropdown_state.load_error_message = None;
        }
        GithubAuthCallState::Unauthed { auth_url } => {
            form.github_dropdown_state.is_loading = false;
            form.github_dropdown_state.auth_url = Some(auth_url);
            form.github_dropdown_state.load_error_message = None;
        }
        GithubAuthCallState::Error { message } => {
            form.github_dropdown_state.is_loading = false;
            form.github_dropdown_state.auth_url = None;
            form.github_dropdown_state.load_error_message = Some(message);
        }
    }
}

fn team_for_test() -> Team {
    Team {
        uid: 123.into(),
        name: "test".to_string(),
        invite_code: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        organization_settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    }
}

fn workspace_for_test(team: &Team) -> Workspace {
    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams: vec![team.clone()],
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_code: None,
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    }
}

#[test]
fn test_environment_form_values_default() {
    let form_state = EnvironmentFormValues::default();

    assert!(form_state.name.is_empty());
    assert!(form_state.description.is_empty());
    assert!(form_state.docker_image.is_empty());
    assert!(form_state.setup_commands.is_empty());
    assert!(form_state.selected_repos.is_empty());
}

#[test]
fn test_edit_mode_initializes_form_state_from_initial_values() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let env_id = SyncId::ClientId(ClientId::new());
            let initial_values = EnvironmentFormValues {
                name: "Test Environment".to_string(),
                description: "A test environment for front end agents".to_string(),
                selected_repos: vec![
                    GithubRepo::new("owner1".to_string(), "repo1".to_string()),
                    GithubRepo::new("owner2".to_string(), "repo2".to_string()),
                ],
                docker_image: "python:3.11".to_string(),
                setup_commands: vec![
                    "pip install -r requirements.txt".to_string(),
                    "pytest".to_string(),
                ],
            };

            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(
                    EnvironmentFormInitArgs::Edit {
                        env_id,
                        initial_values: Box::new(initial_values.clone()),
                    },
                    ctx,
                )
            });

            let form = view_handle.as_ref(ctx);
            assert_eq!(form.form_state.name, initial_values.name);
            assert_eq!(form.form_state.description, initial_values.description);
            assert_eq!(form.form_state.docker_image, initial_values.docker_image);
            assert_eq!(
                form.form_state.setup_commands,
                initial_values.setup_commands
            );
            assert_eq!(
                form.form_state.selected_repos,
                initial_values.selected_repos
            );
        });
    })
}

#[test]
fn test_submit_button_disabled_until_required_fields_present() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        let mut view_handle = None;
        app.update(|ctx| {
            view_handle = Some(ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            }));
        });
        let view_handle = view_handle.expect("UpdateEnvironmentForm handle should be created");

        // Initial create page: missing required fields -> disabled.
        app.update(|ctx| {
            let is_disabled = view_handle
                .as_ref(ctx)
                .submit_button
                .read(ctx, |button, _| button.is_disabled());
            assert!(is_disabled, "Expected submit button disabled initially");
        });

        // Name set (docker image optional) -> enabled.
        app.update(|ctx| {
            view_handle.update(ctx, |form, ctx| {
                form.form_state.name = "My Env".to_string();
                form.update_button_state(ctx);
            });

            let is_disabled = view_handle
                .as_ref(ctx)
                .submit_button
                .read(ctx, |button, _| button.is_disabled());
            assert!(
                !is_disabled,
                "Expected submit button enabled once the name is set"
            );
        });
    })
}

#[test]
fn test_is_valid_requires_only_name() {
    let values = EnvironmentFormValues {
        name: "env".to_string(),
        description: String::new(),
        selected_repos: vec![],
        docker_image: String::new(),
        setup_commands: vec![],
    };

    // The docker image is optional; a name alone is valid.
    assert!(values.is_valid());

    // An empty name is invalid.
    let no_name = EnvironmentFormValues {
        name: "  ".to_string(),
        ..values
    };
    assert!(!no_name.is_valid());
}

#[test]
fn test_empty_docker_image_produces_none_base_image() {
    let values = EnvironmentFormValues {
        name: "env".to_string(),
        description: String::new(),
        selected_repos: vec![],
        docker_image: "  ".to_string(),
        setup_commands: vec![],
    };
    assert_eq!(values.to_ambient_agent_environment().base_image, None);

    let with_image = EnvironmentFormValues {
        docker_image: "ubuntu:latest".to_string(),
        ..values
    };
    assert_eq!(
        with_image.to_ambient_agent_environment().base_image,
        Some(BaseImage::DockerImage("ubuntu:latest".to_string()))
    );
}

#[test]
fn test_edit_mode_allows_saving_environment_without_docker_image() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let env_id = SyncId::ClientId(ClientId::new());
            let initial_values = EnvironmentFormValues {
                name: "No image env".to_string(),
                description: String::new(),
                selected_repos: vec![],
                docker_image: String::new(),
                setup_commands: vec![],
            };

            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(
                    EnvironmentFormInitArgs::Edit {
                        env_id,
                        initial_values: Box::new(initial_values),
                    },
                    ctx,
                )
            });

            let is_disabled = view_handle
                .as_ref(ctx)
                .submit_button
                .read(ctx, |button, _| button.is_disabled());
            assert!(
                !is_disabled,
                "Expected save enabled when editing a no-image environment"
            );
        });
    })
}

#[test]
fn test_render_repos_field_loading_state() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });
            view_handle.update(ctx, |form, _| {
                set_github_auth_call_state(form, GithubAuthCallState::Loading);
            });

            let appearance = Appearance::as_ref(ctx);
            let element = view_handle.as_ref(ctx).render_repos_field(appearance);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("Repo(s)"),
                "Expected 'Repo(s)' label in rendered content: {text_content}"
            );
            assert!(
                text_content.contains("Loading..."),
                "Expected 'Loading...' in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_render_repos_field_authed_state() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });
            view_handle.update(ctx, |form, _| {
                set_github_auth_call_state(form, GithubAuthCallState::Authed);
            });

            let appearance = Appearance::as_ref(ctx);
            let element = view_handle.as_ref(ctx).render_repos_field(appearance);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("Repo(s)"),
                "Expected 'Repo(s)' label in rendered content: {text_content}"
            );
            assert!(
                text_content.contains("Type owner/repo and press Enter"),
                "Expected helper text in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_render_repos_field_auth_required() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });
            view_handle.update(ctx, |form, _| {
                set_github_auth_call_state(
                    form,
                    GithubAuthCallState::unauthed("https://github.com/login/oauth/authorize"),
                );
            });

            let appearance = Appearance::as_ref(ctx);
            let element = view_handle.as_ref(ctx).render_repos_field(appearance);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("Repo(s)"),
                "Expected 'Repo(s)' label in rendered content: {text_content}"
            );
            assert!(
                text_content.contains("Auth with GitHub"),
                "Expected 'Auth with GitHub' in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_render_repos_field_error_state() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });
            view_handle.update(ctx, |form, _| {
                set_github_auth_call_state(
                    form,
                    GithubAuthCallState::error("Failed to load GitHub repositories"),
                );
            });

            let appearance = Appearance::as_ref(ctx);
            let element = view_handle.as_ref(ctx).render_repos_field(appearance);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("Repo(s)"),
                "Expected 'Repo(s)' label in rendered content: {text_content}"
            );
            assert!(
                text_content.contains("Failed to load GitHub repositories"),
                "Expected error message in rendered content: {text_content}"
            );
            assert!(
                text_content.contains("Retry"),
                "Expected 'Retry' in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_repos_field_error_state_allows_manual_repo_entry() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        let mut view_handle = None;
        app.update(|ctx| {
            view_handle = Some(ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            }));
        });
        let view_handle = view_handle.expect("UpdateEnvironmentForm handle should be created");

        app.update(|ctx| {
            view_handle.update(ctx, |form, ctx| {
                set_github_auth_call_state(
                    form,
                    GithubAuthCallState::error("Failed to load GitHub repositories"),
                );
                form.repos_input = "warpdotdev/warp-internal".to_string();
                form.handle_action(&UpdateEnvironmentFormAction::AddRepo, ctx);
            });

            let form = view_handle.as_ref(ctx);
            assert_eq!(form.form_state.selected_repos.len(), 1);
            assert_eq!(form.form_state.selected_repos[0].owner, "warpdotdev");
            assert_eq!(form.form_state.selected_repos[0].repo, "warp-internal");
            assert!(
                form.github_dropdown_state.load_error_message.is_some(),
                "Expected GitHub load error to remain visible after manually adding a repo"
            );
        });
    })
}

#[test]
fn test_render_repos_field_with_selected_repos() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });
            view_handle.update(ctx, |form, _| {
                set_github_auth_call_state(form, GithubAuthCallState::Authed);
                form.form_state.selected_repos = vec![
                    GithubRepo::new("warpdotdev".to_string(), "warp-internal".to_string()),
                    GithubRepo::new("facebook".to_string(), "react".to_string()),
                ];
                form.remove_repo_mouse_states =
                    vec![MouseStateHandle::default(), MouseStateHandle::default()];
            });

            let appearance = Appearance::as_ref(ctx);
            let element = view_handle.as_ref(ctx).render_repos_field(appearance);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("Repo(s)"),
                "Expected 'Repo(s)' label in rendered content: {text_content}"
            );
            assert!(
                text_content.contains("warpdotdev/warp-internal"),
                "Expected 'warpdotdev/warp-internal' in rendered content: {text_content}"
            );
            assert!(
                text_content.contains("facebook/react"),
                "Expected 'facebook/react' in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_authed_repo_input_allows_arbitrary_repo() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        let mut view_handle = None;
        app.update(|ctx| {
            view_handle = Some(ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            }));
        });
        let view_handle = view_handle.expect("UpdateEnvironmentForm handle should be created");

        app.update(|ctx| {
            view_handle.update(ctx, |form, ctx| {
                set_github_auth_call_state(form, GithubAuthCallState::Authed);
                form.github_dropdown_state.available_repos =
                    vec![GithubRepo::new("other".to_string(), "repo".to_string())];
                form.repos_input = "owner/new-repo".to_string();
                form.handle_action(&UpdateEnvironmentFormAction::AddRepo, ctx);
            });

            let form = view_handle.as_ref(ctx);
            assert_eq!(form.form_state.selected_repos.len(), 1);
            assert_eq!(form.form_state.selected_repos[0].owner, "owner");
            assert_eq!(form.form_state.selected_repos[0].repo, "new-repo");
            assert!(form.repos_input.is_empty());
            assert_eq!(form.remove_repo_mouse_states.len(), 1);
        });
    })
}

#[test]
fn test_selected_repos_as_remote_repo_args_formats_owner_repo_strings() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });
            view_handle.update(ctx, |form, _| {
                form.form_state.selected_repos = vec![
                    GithubRepo::new("warpdotdev".to_string(), "warp-internal".to_string()),
                    GithubRepo::new("facebook".to_string(), "react".to_string()),
                ];
            });

            let form = view_handle.as_ref(ctx);
            let args = form.selected_repos_as_remote_repo_args();

            assert_eq!(
                args,
                vec![
                    "warpdotdev/warp-internal".to_string(),
                    "facebook/react".to_string(),
                ]
            );
        });
    })
}

#[test]
fn test_can_suggest_image_for_edit_requires_repos_modified() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        let env_id = SyncId::ClientId(ClientId::new());
        let initial_values = EnvironmentFormValues {
            name: "Env".to_string(),
            description: "".to_string(),
            selected_repos: vec![GithubRepo::new(
                "warpdotdev".to_string(),
                "warp-internal".to_string(),
            )],
            docker_image: "ubuntu:latest".to_string(),
            setup_commands: vec![],
        };

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(
                    EnvironmentFormInitArgs::Edit {
                        env_id,
                        initial_values: Box::new(initial_values),
                    },
                    ctx,
                )
            });

            let form = view_handle.as_ref(ctx);
            assert!(
                !form.can_suggest_image_for_current_repos(),
                "Expected suggest-image to be disabled on edit until repos are modified"
            );

            view_handle.update(ctx, |form, _| {
                form.edit_repos_modified = true;
                form.suggest_image_last_attempt_key = None;
                form.suggest_image_state = SuggestImageState::Idle;
            });

            let form = view_handle.as_ref(ctx);
            assert!(
                form.can_suggest_image_for_current_repos(),
                "Expected suggest-image to be enabled on edit after repos have been modified"
            );
        });
    })
}

#[test]
fn test_can_suggest_image_for_create_does_not_require_repos_modified() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });
            view_handle.update(ctx, |form, _| {
                form.form_state.selected_repos = vec![GithubRepo::new(
                    "warpdotdev".to_string(),
                    "warp-internal".to_string(),
                )];
                form.edit_repos_modified = false;
                form.suggest_image_last_attempt_key = None;
                form.suggest_image_state = SuggestImageState::Idle;
            });

            let form = view_handle.as_ref(ctx);
            assert!(
                form.can_suggest_image_for_current_repos(),
                "Expected suggest-image to be enabled on create when repos exist"
            );
        });
    })
}

#[test]
fn test_render_docker_image_field_shows_suggest_image_button_on_create() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });

            let appearance = Appearance::as_ref(ctx);
            let element = view_handle
                .as_ref(ctx)
                .render_docker_image_field(appearance);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("Suggest image"),
                "Expected suggest-image button text in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_render_docker_image_field_shows_suggest_image_button_on_edit() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let env_id = SyncId::ClientId(ClientId::new());
            let initial_values = EnvironmentFormValues {
                name: "Env".to_string(),
                description: "".to_string(),
                selected_repos: vec![],
                docker_image: "ubuntu:latest".to_string(),
                setup_commands: vec![],
            };

            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(
                    EnvironmentFormInitArgs::Edit {
                        env_id,
                        initial_values: Box::new(initial_values),
                    },
                    ctx,
                )
            });

            let appearance = Appearance::as_ref(ctx);
            let element = view_handle
                .as_ref(ctx)
                .render_docker_image_field(appearance);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("Suggest image"),
                "Expected suggest-image button text in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_render_docker_image_field_shows_generating_state() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });
            view_handle.update(ctx, |form, _| {
                form.form_state.selected_repos = vec![GithubRepo::new(
                    "warpdotdev".to_string(),
                    "warp-internal".to_string(),
                )];
                let key = form
                    .selected_repos_key()
                    .expect("Expected repos key to exist");
                form.suggest_image_state = SuggestImageState::Loading { key };
            });

            let appearance = Appearance::as_ref(ctx);
            let element = view_handle
                .as_ref(ctx)
                .render_docker_image_field(appearance);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("Generating"),
                "Expected generating state in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_render_docker_image_field_shows_custom_image_warning() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });
            view_handle.update(ctx, |form, _| {
                form.form_state.selected_repos = vec![GithubRepo::new(
                    "warpdotdev".to_string(),
                    "warp-internal".to_string(),
                )];
                let key = form
                    .selected_repos_key()
                    .expect("Expected repos key to exist");
                form.suggest_image_state = SuggestImageState::Success {
                    key,
                    needs_custom_image: true,
                    reason: "No matching base image".to_string(),
                };
            });

            let appearance = Appearance::as_ref(ctx);
            let element = view_handle
                .as_ref(ctx)
                .render_docker_image_field(appearance);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains("custom Docker image"),
                "Expected custom image messaging in rendered content: {text_content}"
            );
            assert!(
                text_content.contains("No matching base image"),
                "Expected reason text in rendered content: {text_content}"
            );
            assert!(
                text_content.contains("Launch agent"),
                "Expected 'Launch agent' action in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_render_docker_image_field_shows_github_auth_required_message() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });
            view_handle.update(ctx, |form, _| {
                form.form_state.selected_repos = vec![GithubRepo::new(
                    "warpdotdev".to_string(),
                    "warp-internal".to_string(),
                )];
                let key = form
                    .selected_repos_key()
                    .expect("Expected repos key to exist");
                form.suggest_image_state = SuggestImageState::AuthRequired {
                    key,
                    auth_url: "https://github.com/login/oauth/authorize".to_string(),
                };
            });

            let appearance = Appearance::as_ref(ctx);
            let element = view_handle
                .as_ref(ctx)
                .render_docker_image_field(appearance);
            let text_content = element.debug_text_content().unwrap_or_default();

            assert!(
                text_content.contains(
                    "You need to grant access to your GitHub repos to suggest a Docker image"
                ),
                "Expected GitHub auth required message in rendered content: {text_content}"
            );
            assert!(
                text_content.contains("Authenticate"),
                "Expected 'Authenticate' action in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_create_environment_form_with_team_can_toggle_share_with_team_and_renders_warning_when_disabled(
) {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let team = team_for_test();
            let workspace = workspace_for_test(&team);
            let workspace_uid = workspace.uid;

            UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                user_workspaces.update_workspaces(vec![workspace], ctx);
                user_workspaces.set_current_workspace_uid(workspace_uid, ctx);
            });

            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });

            assert!(
                view_handle.as_ref(ctx).share_with_team,
                "Expected share_with_team to default to true when user has a team"
            );

            let element = view_handle.as_ref(ctx).render(ctx);
            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                text_content.contains("Share with team"),
                "Expected 'Share with team' checkbox label in rendered content: {text_content}"
            );
            assert!(
                !text_content.contains(
                    "Personal environments cannot be used with external integrations or team API keys",
                ),
                "Did not expect the warning to render when share_with_team is enabled: {text_content}"
            );

            view_handle.update(ctx, |view, ctx| {
                view.handle_action(&UpdateEnvironmentFormAction::ToggleShareWithTeam, ctx);
            });

            assert!(
                !view_handle.as_ref(ctx).share_with_team,
                "Expected share_with_team to be disabled after toggle"
            );

            let element = view_handle.as_ref(ctx).render(ctx);
            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                text_content.contains(
                    "Personal environments cannot be used with external integrations or team API keys",
                ),
                "Expected the warning to render when share_with_team is disabled: {text_content}"
            );
        });
    })
}

#[test]
fn test_create_environment_form_without_team_does_not_render_checkbox_and_defaults_disabled() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx)
            });

            assert!(
                !view_handle.as_ref(ctx).share_with_team,
                "Expected share_with_team to default to false when user has no team"
            );

            let element = view_handle.as_ref(ctx).render(ctx);
            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                !text_content.contains("Share with team"),
                "Did not expect 'Share with team' checkbox label in rendered content: {text_content}"
            );
        });
    })
}

#[test]
fn test_environment_form_copy_orchestration_modal_overrides_settings_defaults() {
    let default_copy = EnvironmentFormCopy::default();
    let orchestration_copy = EnvironmentFormCopy::orchestration_modal();

    assert_eq!(default_copy.name_placeholder, "Environment name");
    assert_eq!(default_copy.docker_image_label, "Docker image reference");
    assert!(default_copy.show_description_character_count);

    assert_eq!(orchestration_copy.name_placeholder, "e.g., dev-env");
    assert_eq!(
        orchestration_copy.repos_placeholder_authed,
        "Browse GitHub repos..."
    );
    assert_eq!(orchestration_copy.docker_image_label, "Docker image");
    assert_eq!(
        orchestration_copy.docker_image_placeholder,
        "e.g., node:20-alpine"
    );
    assert_eq!(
        orchestration_copy.setup_commands_placeholder,
        "e.g., node start"
    );
    assert_eq!(
        orchestration_copy.setup_commands_helper,
        "Press Enter or click the submit button to add each command."
    );
    assert!(!orchestration_copy.show_description_character_count);
}

#[test]
fn test_orchestration_modal_form_configuration_renders_footer_actions_without_team_controls() {
    App::test((), |mut app| async move {
        init_update_environment_form_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let team = team_for_test();
            let workspace = workspace_for_test(&team);
            let workspace_uid = workspace.uid;

            UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                user_workspaces.update_workspaces(vec![workspace], ctx);
                user_workspaces.set_current_workspace_uid(workspace_uid, ctx);
            });

            let view_handle = ctx.add_typed_action_view(window_id, |ctx| {
                let mut form =
                    UpdateEnvironmentForm::new_for_test(EnvironmentFormInitArgs::Create, ctx);
                form.set_show_header(false, ctx);
                form.configure_for_orchestration_modal(ctx);
                form
            });

            let form = view_handle.as_ref(ctx);
            assert!(!form.show_header);
            assert!(form.show_footer_cancel_button);
            assert!(!form.show_share_with_team_controls);
            assert_eq!(form.field_spacing, 10.);
            assert_eq!(form.description_height, 52.);
            assert!(!form.show_repo_helper_text);
            assert!(!form.copy.show_description_character_count);

            let submit_button = form.submit_button.clone();
            let cancel_button = form.cancel_button.clone();

            let submit_text = submit_button
                .as_ref(ctx)
                .render(ctx)
                .debug_text_content()
                .unwrap_or_default();
            let cancel_text = cancel_button
                .as_ref(ctx)
                .render(ctx)
                .debug_text_content()
                .unwrap_or_default();
            assert!(
                submit_text.contains("Create environment"),
                "Expected footer submit label in rendered content: {submit_text}"
            );
            assert!(
                cancel_text.contains("Cancel"),
                "Expected footer cancel action in rendered content: {cancel_text}"
            );

            let text_content = view_handle
                .as_ref(ctx)
                .render(ctx)
                .debug_text_content()
                .unwrap_or_default();
            assert!(
                !text_content.contains("Share with team"),
                "Did not expect team-sharing controls in orchestration modal form: {text_content}"
            );
            assert!(
                !text_content.contains("0 / 240 characters"),
                "Did not expect settings character count in orchestration modal form: {text_content}"
            );
            assert!(
                !text_content.contains("Type owner/repo and press Enter"),
                "Did not expect settings repo helper text in orchestration modal form: {text_content}"
            );
        });
    })
}

#[test]
fn test_parse_docker_hub_url_bare_owner_repo() {
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("warp/base-image"),
        Some("https://hub.docker.com/r/warp/base-image".to_string())
    );
}

#[test]
fn test_parse_docker_hub_url_with_tag() {
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("warp/base-image:latest"),
        Some("https://hub.docker.com/r/warp/base-image".to_string())
    );
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("warp/base-image:v1.2.3"),
        Some("https://hub.docker.com/r/warp/base-image".to_string())
    );
}

#[test]
fn test_parse_docker_hub_url_with_digest() {
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("warp/base-image@sha256:abc123"),
        Some("https://hub.docker.com/r/warp/base-image".to_string())
    );
}

#[test]
fn test_parse_docker_hub_url_explicit_docker_io() {
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("docker.io/warp/base-image"),
        Some("https://hub.docker.com/r/warp/base-image".to_string())
    );
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("docker.io/warp/base-image:latest"),
        Some("https://hub.docker.com/r/warp/base-image".to_string())
    );
}

#[test]
fn test_parse_docker_hub_url_explicit_index_docker_io() {
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("index.docker.io/warp/base-image"),
        Some("https://hub.docker.com/r/warp/base-image".to_string())
    );
}

#[test]
fn test_parse_docker_hub_url_official_image() {
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("python"),
        Some("https://hub.docker.com/_/python".to_string())
    );
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("python:3.11"),
        Some("https://hub.docker.com/_/python".to_string())
    );
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("node:20-alpine"),
        Some("https://hub.docker.com/_/node".to_string())
    );
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("golang"),
        Some("https://hub.docker.com/_/golang".to_string())
    );
}

#[test]
fn test_parse_docker_hub_url_official_image_explicit_library_prefix() {
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("docker.io/library/python"),
        Some("https://hub.docker.com/_/python".to_string())
    );
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("index.docker.io/library/node:20"),
        Some("https://hub.docker.com/_/node".to_string())
    );
}

#[test]
fn test_parse_docker_hub_url_other_registry_returns_none() {
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("ghcr.io/warp/base-image"),
        None
    );
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("gcr.io/project/image"),
        None
    );
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("quay.io/owner/repo"),
        None
    );
}

#[test]
fn test_parse_docker_hub_url_empty_or_whitespace() {
    assert_eq!(UpdateEnvironmentForm::parse_docker_hub_url(""), None);
    assert_eq!(UpdateEnvironmentForm::parse_docker_hub_url("   "), None);
}

#[test]
fn test_parse_docker_hub_url_trims_whitespace() {
    assert_eq!(
        UpdateEnvironmentForm::parse_docker_hub_url("  warp/base-image  "),
        Some("https://hub.docker.com/r/warp/base-image".to_string())
    );
}
