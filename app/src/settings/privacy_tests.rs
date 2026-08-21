use std::cell::Cell;
use std::rc::Rc;

use settings::schema::SettingSchemaEntry;
use settings::{Setting, SettingSurfaces, SettingsMode};
use warpui::{App, Entity, SingletonEntity};

use super::{
    IsCloudConversationStorageEnabled, IsCrashReportingEnabled, IsTelemetryEnabled, PrivacySettings,
};
use crate::test_util::settings::initialize_settings_for_tests;

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

/// A stand-in for a view like `PrivacyPageView`, which re-renders whenever it observes
/// `PrivacySettings` being notified.
struct NotifyProbe;

impl Entity for NotifyProbe {
    type Event = ();
}

/// `UserWorkspaces::notify_and_emit_teams_changed` calls
/// `set_is_enterprise_secret_redaction_enabled` on every teams refresh, whether or not the
/// enabled bit actually changed -- a refresh can update only the enterprise regex list (read
/// live from `UserWorkspaces` at render time), which an observer only learns about via this
/// notification. A same-enabled call must still notify, or an already-open privacy page goes
/// stale exactly when an admin edits the regex list.
#[test]
fn set_is_enterprise_secret_redaction_enabled_notifies_even_when_unchanged() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(PrivacySettings::mock);

        let notify_count = Rc::new(Cell::new(0u32));
        let probe_notify_count = notify_count.clone();
        let _probe = app.add_model(|ctx| {
            ctx.observe(&PrivacySettings::handle(ctx), move |_, _, _| {
                probe_notify_count.set(probe_notify_count.get() + 1);
            });
            NotifyProbe
        });

        PrivacySettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.set_is_enterprise_secret_redaction_enabled(true, ctx);
        });
        assert_eq!(
            notify_count.get(),
            1,
            "enabling for the first time should notify observers"
        );

        // Simulate a metadata refresh that leaves `enabled` unchanged.
        PrivacySettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.set_is_enterprise_secret_redaction_enabled(true, ctx);
        });
        assert_eq!(
            notify_count.get(),
            2,
            "a same-enabled refresh must still notify observers"
        );
    })
}
