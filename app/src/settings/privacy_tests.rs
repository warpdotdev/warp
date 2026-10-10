use settings::schema::SettingSchemaEntry;
use settings::{
    PrivatePreferences, PublicPreferences, Setting, SettingSurfaces, SettingsManager, SettingsMode,
};
use warpui::App;

use super::*;

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

fn register_settings_singletons(app: &mut App) {
    // The setting framework's `set_value` calls reach for these
    // singletons; PrivacySettings::mock doesn't register them itself.
    app.add_singleton_model(|_| {
        PublicPreferences::new(Box::<
            warpui_extras::user_preferences::in_memory::InMemoryPreferences,
        >::default())
    });
    app.add_singleton_model(|_| {
        PrivatePreferences::new(Box::<
            warpui_extras::user_preferences::in_memory::InMemoryPreferences,
        >::default())
    });
    app.add_singleton_model(|_| SettingsManager::default());
}

fn count_secret_regex_list_events(
    app: &mut App,
    model: &warpui::ModelHandle<PrivacySettings>,
    mutate: impl FnOnce(&mut PrivacySettings, &mut ModelContext<PrivacySettings>),
) -> usize {
    let (sender, receiver) = async_channel::unbounded();
    // Subscribe at the app level rather than from the model itself:
    // `ModelContext::subscribe_to_model` disallows self-subscription, since
    // `emit_event` removes the subscriber from `app.models` while dispatching.
    app.update(|ctx| {
        ctx.subscribe_to_model(model, move |_, event, _| {
            if matches!(
                event,
                PrivacySettingsChangedEvent::CustomSecretRegexList { .. }
            ) {
                let _ = sender.try_send(());
            }
        });
    });

    model.update(app, |settings, ctx| mutate(settings, ctx));

    let mut count = 0;
    while receiver.try_recv().is_ok() {
        count += 1;
    }
    count
}

#[test]
fn add_all_recommended_regex_emits_custom_secret_regex_list_event() {
    // Adding the recommended defaults mutates `user_secret_regex_list`, so it
    // must emit `CustomSecretRegexList` for `CustomSecretRegexUpdater` to
    // recompile the in-memory `SECRETS_REGEX` DFA.
    App::test((), |mut app| async move {
        register_settings_singletons(&mut app);
        let model = app.add_model(PrivacySettings::mock);

        let event_count = count_secret_regex_list_events(&mut app, &model, |settings, ctx| {
            settings.add_all_recommended_regex(ctx);
        });

        assert_eq!(
            event_count, 1,
            "add_all_recommended_regex must emit exactly one CustomSecretRegexList event"
        );
    });
}

#[test]
fn add_all_recommended_regex_when_already_populated_does_not_emit() {
    // Early-return guard: if every default is already present, no
    // mutation happens and we shouldn't fire an event for nothing.
    App::test((), |mut app| async move {
        register_settings_singletons(&mut app);
        let model = app.add_model(PrivacySettings::mock);

        // First call adds the defaults and fires once.
        let first_count = count_secret_regex_list_events(&mut app, &model, |settings, ctx| {
            settings.add_all_recommended_regex(ctx);
        });
        assert_eq!(first_count, 1);

        // Second call is a no-op (early-return at `num_existing_regexes
        // == new.len()`) and must NOT emit.
        let second_count = count_secret_regex_list_events(&mut app, &model, |settings, ctx| {
            settings.add_all_recommended_regex(ctx);
        });
        assert_eq!(
            second_count, 0,
            "no-op call must not spuriously emit CustomSecretRegexList"
        );
    });
}

#[test]
fn remove_user_secret_regex_emits_custom_secret_regex_list_event() {
    App::test((), |mut app| async move {
        register_settings_singletons(&mut app);
        let model = app.add_model(PrivacySettings::mock);

        // Seed the list so there's something to remove.
        model.update(&mut app, |settings, ctx| {
            settings.add_all_recommended_regex(ctx);
        });

        let event_count = count_secret_regex_list_events(&mut app, &model, |settings, ctx| {
            settings.remove_user_secret_regex(&0, ctx);
        });

        assert_eq!(
            event_count, 1,
            "remove_user_secret_regex must emit exactly one CustomSecretRegexList event"
        );
    });
}
