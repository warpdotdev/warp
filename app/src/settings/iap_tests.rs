use settings::{PrivatePreferences, Setting, SettingsManager, SyncToCloud};
use warpui::SingletonEntity;
use warpui_extras::user_preferences::in_memory::InMemoryPreferences;

use super::*;

#[test]
fn routing_is_default_off_and_device_local() {
    assert!(!*IapRoutingEnabled::new(None));
    assert!(IapRoutingEnabled::is_private());
    assert_eq!(IapRoutingEnabled::sync_to_cloud(), SyncToCloud::Never);
}

#[test]
fn routing_choice_survives_reloading_from_storage() {
    warpui::App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| PrivatePreferences::new(Box::<InMemoryPreferences>::default()));
            ctx.add_singleton_model(|_| SettingsManager::default());
            IapRoutingSettings::register(ctx);
            IapRoutingSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.routing_enabled.set_value(true, ctx).unwrap();
            });
            assert!(*IapRoutingEnabled::new_from_storage(ctx));
            IapRoutingSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.routing_enabled.set_value(false, ctx).unwrap();
            });
            assert!(!*IapRoutingEnabled::new_from_storage(ctx));
        });
    });
}
