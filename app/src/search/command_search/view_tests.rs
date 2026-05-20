use warpui::{platform::WindowStyle, App};

use crate::{
    cloud_object::model::persistence::CloudModel,
    network::NetworkStatus,
    server::{
        cloud_objects::{listener::Listener, update_manager::UpdateManager},
        server_api::ServerApiProvider,
        sync_queue::SyncQueue,
        telemetry::context_provider::AppTelemetryContextProvider,
    },
    settings_view::keybindings::KeybindingChangedNotifier,
    system::SystemStats,
    test_util::settings::initialize_settings_for_tests,
    workspaces::{
        team_tester::TeamTesterStatus, update_manager::TeamUpdateManager,
        user_workspaces::UserWorkspaces,
    },
};

use super::*;

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(TeamUpdateManager::mock);
    app.add_singleton_model(Listener::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| ResizableData::default());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
}

#[test]
fn test_render_view() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_window_id, _view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            CommandSearchView::new(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
        });

        app.update(|_| {
            // This will force a redraw of the window, which lays out the
            // window, including the command search view.
        });
    });
}
