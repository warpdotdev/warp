use warp_core::ui::appearance::Appearance;
use warpui::App;
use warpui::platform::WindowStyle;

use super::DrivePanel;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::cloud_object::Space;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::model::view::CloudViewModel;
use crate::drive::index::DriveIndexSection;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal::resizable_data::ResizableData;
use crate::terminal::shared_session::permissions_manager::SessionPermissionsManager;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{ASSETS, ObjectActions};

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(|_| ResizableData::default());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(CloudViewModel::mock);
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(SessionPermissionsManager::new);
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
}

#[test]
fn test_warp_drive_sections_with_no_team() {
    App::test(ASSETS, |mut app| async move {
        initialize_app(&mut app);

        // Instead of being in the panel module and depending on DrivePanel, this test should be in the index module.
        // It happens to be here for the time being because `DriveIndex` depends on `DrivePanel` calling the `initialize_section_states` method.
        // Ideally, the constructor should handle the necessary initialization but for now this functional test asserts that the drive index is setup.
        let (_, panel) = app.add_window(WindowStyle::NotStealFocus, DrivePanel::new);

        let index = panel.read(&app, |panel, _| panel.index_view.clone());
        index.read(&app, |index, _| {
            let sections = index.sections();
            assert_eq!(sections.len(), 2);
            assert_eq!(sections[0], DriveIndexSection::CreateATeam);
            assert_eq!(sections[1], DriveIndexSection::Space(Space::Personal))
        });
    })
}
