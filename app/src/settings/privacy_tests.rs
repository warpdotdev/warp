use std::sync::Arc;
use std::time::Duration;

use settings::schema::SettingSchemaEntry;
use settings::{Setting, SettingSurfaces, SettingsMode};
use warpui::{App, SingletonEntity};

use super::{
    IsCloudConversationStorageEnabled, IsCrashReportingEnabled, IsTelemetryEnabled, PrivacySettings,
};
use crate::auth::auth_state::AuthState;
use crate::auth::user::{PrincipalType, User};
use crate::server::server_api::auth::MockAuthClient;

#[test]
fn privacy_settings_apply_to_gui_and_tui() {
    for storage_key in [
        IsTelemetryEnabled::toml_key(),
        IsCrashReportingEnabled::toml_key(),
        IsCloudConversationStorageEnabled::toml_key(),
    ] {
        let entry = inventory::iter::<SettingSchemaEntry>
            .into_iter()
            .find(|entry| entry.storage_key == storage_key)
            .unwrap_or_else(|| panic!("missing schema entry for {storage_key}"));
        let surfaces = (entry.surfaces_fn)();

        assert_eq!(surfaces, SettingSurfaces::ALL, "{storage_key}");
        assert!(surfaces.includes(SettingsMode::Gui), "{storage_key}");
        assert!(surfaces.includes(SettingsMode::Tui), "{storage_key}");
    }
}

#[test]
fn service_account_keeps_privacy_settings_sync_inert() {
    App::test((), |mut app| async move {
        let auth_state = Arc::new(AuthState::new_for_test());
        let mut user = User::test();
        user.principal_type = PrincipalType::ServiceAccount;
        auth_state.set_user(Some(user));

        let mut auth_client = MockAuthClient::new();
        auth_client.expect_get_user_settings().times(0);
        app.add_singleton_model(move |ctx| {
            let mut settings = PrivacySettings::mock(ctx);
            settings.auth_state = auth_state;
            settings.auth_client = Arc::new(auth_client);
            settings
        });

        PrivacySettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.fetch_or_update_settings(ctx);
            settings.maybe_sync_with_warp_drive_prefs(ctx);
        });

        warpui::r#async::Timer::after(Duration::from_millis(10)).await;
    })
}
