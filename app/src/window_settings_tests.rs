use settings::{PrivatePreferences, PublicPreferences, Setting, SettingsManager};
use warp_core::features::FeatureFlag;
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::platform::WindowBackdrop;
use warpui::{App, AppContext, SingletonEntity};
use warpui_extras::user_preferences;

use super::{
    BackgroundBackdrop, LegacyOverrideBlurTexture, WindowSettings,
    migrate_legacy_background_backdrop, stage_legacy_background_backdrop,
};

fn initialize_settings(
    legacy_value: bool,
    background_backdrop: Option<WindowBackdrop>,
    ctx: &mut AppContext,
) {
    ctx.add_singleton_model(|_| {
        PublicPreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    ctx.add_singleton_model(|_| {
        PrivatePreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    ctx.add_singleton_model(|_| SettingsManager::default());

    ctx.private_user_preferences()
        .write_value(
            LegacyOverrideBlurTexture::storage_key(),
            serde_json::to_string(&legacy_value).unwrap(),
        )
        .unwrap();
    if let Some(background_backdrop) = background_backdrop {
        ctx.private_user_preferences()
            .write_value(
                BackgroundBackdrop::storage_key(),
                serde_json::to_string(&background_backdrop).unwrap(),
            )
            .unwrap();
    }

    WindowSettings::register(ctx);
}

#[test]
fn legacy_true_migrates_to_acrylic_after_initial_load() {
    App::test((), |mut app| async move {
        let _settings_file = FeatureFlag::SettingsFile.override_enabled(false);
        app.update(|ctx| initialize_settings(true, None, ctx));

        app.update(stage_legacy_background_backdrop);
        app.read(|ctx| {
            let backdrop = &WindowSettings::as_ref(ctx).background_backdrop;
            assert_eq!(backdrop.value(), &WindowBackdrop::Acrylic);
            assert!(!backdrop.is_value_explicitly_set());
            assert!(
                ctx.private_user_preferences()
                    .read_value(BackgroundBackdrop::storage_key())
                    .unwrap()
                    .is_none()
            );
        });

        app.update(migrate_legacy_background_backdrop);
        app.read(|ctx| {
            let backdrop = &WindowSettings::as_ref(ctx).background_backdrop;
            assert_eq!(backdrop.value(), &WindowBackdrop::Acrylic);
            assert!(backdrop.is_value_explicitly_set());
            assert!(
                ctx.private_user_preferences()
                    .read_value(BackgroundBackdrop::storage_key())
                    .unwrap()
                    .is_some()
            );
        });
    });
}

#[test]
fn legacy_false_leaves_background_backdrop_unset() {
    App::test((), |mut app| async move {
        let _settings_file = FeatureFlag::SettingsFile.override_enabled(false);
        app.update(|ctx| initialize_settings(true, None, ctx));

        app.update(stage_legacy_background_backdrop);
        app.read(|ctx| {
            let backdrop = &WindowSettings::as_ref(ctx).background_backdrop;
            assert_eq!(backdrop.value(), &WindowBackdrop::Acrylic);
            assert!(!backdrop.is_value_explicitly_set());
            assert!(
                ctx.private_user_preferences()
                    .read_value(BackgroundBackdrop::storage_key())
                    .unwrap()
                    .is_none()
            );
        });

        WindowSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .legacy_override_blur_texture
                .set_value_from_cloud_sync(false, ctx)
                .unwrap();
        });
        app.update(migrate_legacy_background_backdrop);

        app.read(|ctx| {
            let backdrop = &WindowSettings::as_ref(ctx).background_backdrop;
            assert_eq!(backdrop.value(), &WindowBackdrop::None);
            assert!(!backdrop.is_value_explicitly_set());
            assert!(
                ctx.private_user_preferences()
                    .read_value(BackgroundBackdrop::storage_key())
                    .unwrap()
                    .is_none()
            );
        });
    });
}

#[test]
fn explicit_background_backdrop_is_not_overwritten() {
    App::test((), |mut app| async move {
        let _settings_file = FeatureFlag::SettingsFile.override_enabled(false);
        app.update(|ctx| initialize_settings(true, Some(WindowBackdrop::Mica), ctx));

        app.update(stage_legacy_background_backdrop);
        app.update(migrate_legacy_background_backdrop);

        app.read(|ctx| {
            let backdrop = &WindowSettings::as_ref(ctx).background_backdrop;
            assert_eq!(backdrop.value(), &WindowBackdrop::Mica);
            assert!(backdrop.is_value_explicitly_set());
        });
    });
}
