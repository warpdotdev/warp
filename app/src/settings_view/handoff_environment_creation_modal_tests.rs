use super::*;

use crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::server_api::ServerApiProvider;
use crate::server::{cloud_objects::update_manager::UpdateManager, sync_queue::SyncQueue};
use crate::settings::PrivacySettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;
use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::App;

fn init_handoff_modal_test_models(app: &mut App) {
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
fn test_modal_focuses_environment_name_editor_when_focused() {
    App::test((), |mut app| async move {
        init_handoff_modal_test_models(&mut app);

        let (window_id, modal) = app.add_window(
            WindowStyle::NotStealFocus,
            HandoffEnvironmentCreationModal::new,
        );

        modal.update(&mut app, |modal, ctx| {
            modal.show(ctx);
            ctx.focus_self();
        });

        let name_editor_id = modal.read(&app, |modal, ctx| {
            modal
                .environment_form
                .as_ref(ctx)
                .name_editor_for_test()
                .id()
        });

        assert_eq!(
            app.focused_view_id(window_id),
            Some(name_editor_id),
            "Expected focusing the popup modal to autofocus the environment name editor"
        );
    })
}
