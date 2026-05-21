use warp_core::ui::appearance::Appearance;
use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{
    AddSingletonModel, App, AppContext, Element, Entity, TypedActionView, View, WindowId,
};

use super::CreateEnvironmentModal;
use crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::settings::PrivacySettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;

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

fn init_create_environment_modal_test_models(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(|_| GitHubAuthNotifier::new());
}

#[test]
fn test_create_environment_modal_uses_orchestration_form_configuration() {
    App::test((), |mut app| async move {
        init_create_environment_modal_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, CreateEnvironmentModal::new);
            let modal = view_handle.as_ref(ctx);

            assert!(
                modal
                    .handoff_modal
                    .as_ref(ctx)
                    .uses_orchestration_form_configuration_for_test(ctx),
                "Expected CreateEnvironmentModal to construct the handoff modal with orchestration form configuration"
            );
        });
    })
}
