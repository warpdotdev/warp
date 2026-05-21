use anyhow::Result;
use warpui::{AppContext, SingletonEntity};

use crate::manager::SettingsManager;
use crate::{Setting, SupportedPlatforms, SyncToCloud};

use crate::*;

define_settings_group!(TestSettings, settings: [
    simple_setting: SimpleSetting {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "test.simple_setting",
    },
    key_override_setting: KeyOverrideSetting {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "KeyIsOverridden",
    },
    hierarchy_flag: HierarchyFlag {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "test_section.hierarchy_flag",
    },
]);

pub fn init_and_register_preferences(ctx: &mut AppContext) {
    ctx.add_singleton_model(move |_| {
        crate::PublicPreferences::new(Box::<
            warpui_extras::user_preferences::in_memory::InMemoryPreferences,
        >::default())
    });
    ctx.add_singleton_model(move |_| {
        crate::PrivatePreferences::new(Box::<
            warpui_extras::user_preferences::in_memory::InMemoryPreferences,
        >::default())
    });
}

/// This is a helper structure that listens to events generated when
/// SimpleSetting is changed.
struct EventListener {
    got_event: bool,
}

impl EventListener {
    fn new(ctx: &mut warpui::ModelContext<Self>) -> Self {
        let test_settings = TestSettings::handle(ctx);
        ctx.subscribe_to_model(&test_settings, |me, event, _ctx| {
            // Update our internal state if we get a change event for
            // SimpleSetting.
            if matches!(event, TestSettingsChangedEvent::SimpleSetting { .. }) {
                me.got_event = true;
            }
        });

        Self { got_event: false }
    }
}

impl warpui::Entity for EventListener {
    type Event = ();
}

#[test]
fn test_setting_name_is_struct_name() {
    assert_eq!(SimpleSetting::setting_name(), "SimpleSetting");
}

#[test]
fn test_default_storage_key_is_setting_name() {
    assert_eq!(SimpleSetting::storage_key(), "SimpleSetting");
}

#[test]
fn test_can_override_storage_key() {
    assert_ne!(
        KeyOverrideSetting::setting_name(),
        KeyOverrideSetting::storage_key()
    );
    assert_eq!(KeyOverrideSetting::storage_key(), "KeyIsOverridden");
}

#[test]
fn test_set_value_raises_changed_event_no_save() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        // Register our TestSettings settings group with the app.
        TestSettings::register(&mut app);

        // Register a model which subscribes to change events on TestSettings.
        let handle = app.add_model(EventListener::new);

        // Make sure that we haven't received any events yet.
        app.read(|ctx| {
            assert!(!handle.as_ref(ctx).got_event);
        });

        // Modify the value of the setting.  This should produce an event that
        // is handled by our EventListener model.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings.simple_setting.set_value(true, ctx);
            });
        });

        // Check that the EventListener model received the event.
        app.read(|ctx| {
            assert!(handle.as_ref(ctx).got_event);
        });
    });
}

#[test]
fn test_set_value_raises_changed_event_save() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        // Register our TestSettings settings group with the app.
        TestSettings::register(&mut app);

        // Register a model which subscribes to change events on TestSettings.
        let handle = app.add_model(EventListener::new);

        // Make sure that we haven't received any events yet.
        app.read(|ctx| {
            assert!(!handle.as_ref(ctx).got_event);
        });

        // Modify the value of the setting.  This should produce an event that
        // is handled by our EventListener model.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                assert!(test_settings.simple_setting.set_value(true, ctx).is_ok());
            });
        });

        // Check that the EventListener model received the event.
        app.read(|ctx| {
            assert!(handle.as_ref(ctx).got_event);
        });
    });
}

#[test]
fn test_save_and_load_lifecycle() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        // Register our TestSettings settings group with the app.
        TestSettings::register(&mut app);

        // Confirm that the initial value of the setting is its default value.
        let default_value = SimpleSetting::default_value();
        app.read(|ctx| {
            assert_eq!(
                *TestSettings::as_ref(ctx).simple_setting.value(),
                default_value
            );
        });

        // Update the value of the setting.
        let new_value = !default_value;
        TestSettings::handle(&app).update(&mut app, |test_settings, ctx| {
            assert!(
                test_settings
                    .simple_setting
                    .set_value(new_value, ctx)
                    .is_ok()
            );
        });

        // Verify that the stored value is correct by creating a new instance of
        // the setting which is initialized from the value in storage.
        app.update(|ctx| {
            let fresh_simple_setting = SimpleSetting::new_from_storage(ctx);
            assert_eq!(*fresh_simple_setting.value(), new_value);
            assert_ne!(*fresh_simple_setting.value(), default_value);
        });
    });
}

#[test]
fn test_toggleable_setting() -> Result<()> {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        // Register our TestSettings settings group with the app.
        TestSettings::register(&mut app);

        // Confirm that the initial value of the setting is its default value.
        let default_value = SimpleSetting::default_value();
        app.read(|ctx| {
            assert_eq!(
                *TestSettings::as_ref(ctx).simple_setting.value(),
                default_value
            );
        });

        // Toggle the value of the setting and verify that the returned value
        // is not the same as the initial value.
        let new_value = TestSettings::handle(&app).update(&mut app, |test_settings, ctx| {
            test_settings.simple_setting.toggle_and_save_value(ctx)
        })?;
        assert_ne!(new_value, default_value);

        // Verify that the in-memory value is correct when reading it back out
        // of the setting.
        app.read(|ctx| {
            assert_eq!(*TestSettings::as_ref(ctx).simple_setting.value(), new_value);
        });

        // Verify that the stored value is correct by creating a new instance of
        // the setting which is initialized from the value in storage.
        app.update(|ctx| {
            let fresh_simple_setting = SimpleSetting::new_from_storage(ctx);
            assert_eq!(*fresh_simple_setting.value(), new_value);
            assert_ne!(*fresh_simple_setting.value(), default_value);
        });

        Ok(())
    })
}

#[test]
fn test_explicit_value_tracking_with_none() {
    // Test that settings created with None are not marked as explicitly set
    let setting = SimpleSetting::new(None);
    assert!(!setting.is_value_explicitly_set());
    assert_eq!(*setting.value(), SimpleSetting::default_value());
}

#[test]
fn test_explicit_value_tracking_with_some() {
    // Test that settings created with Some(value) are marked as explicitly set
    let setting = SimpleSetting::new(Some(true));
    assert!(setting.is_value_explicitly_set());
    assert!(*setting.value());
}

#[test]
fn test_explicit_value_tracking_after_set_value() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        // Register our TestSettings settings group with the app.
        TestSettings::register(&mut app);

        // Initially, the setting should not be explicitly set (default value)
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(!settings.simple_setting.is_value_explicitly_set());
        });

        // After setting a value, it should be marked as explicitly set
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings.simple_setting.set_value(true, ctx);
            });
        });

        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(settings.simple_setting.is_value_explicitly_set());
            assert!(*settings.simple_setting.value());
        });
    });
}

#[test]
fn test_explicit_value_tracking_after_clear_value() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        // Register our TestSettings settings group with the app.
        TestSettings::register(&mut app);

        // Set a value first
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings.simple_setting.set_value(true, ctx);
            });
        });

        // Verify it's marked as explicitly set
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(settings.simple_setting.is_value_explicitly_set());
        });

        // Clear the value
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings.simple_setting.clear_value(ctx);
            });
        });

        // After clearing, it should no longer be marked as explicitly set
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(!settings.simple_setting.is_value_explicitly_set());
            assert_eq!(
                *settings.simple_setting.value(),
                SimpleSetting::default_value()
            );
        });
    });
}

#[test]
fn test_explicit_value_tracking_from_storage() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        // Store a value in preferences manually, using the same backend
        // that new_from_storage will read from.
        app.update(|ctx| {
            let prefs = SimpleSetting::preferences_for_setting(ctx);
            let _ = SimpleSetting::write_to_preferences(&true, prefs);
        });

        // Create a new setting from storage - it should be marked as explicitly set
        app.update(|ctx| {
            let setting_from_storage = SimpleSetting::new_from_storage(ctx);
            assert!(setting_from_storage.is_value_explicitly_set());
            assert!(*setting_from_storage.value());
        });

        // Clear the value from preferences
        app.update(|ctx| {
            let prefs = SimpleSetting::preferences_for_setting(ctx);
            let _ = SimpleSetting::clear_from_preferences(prefs);
        });

        // Create a new setting from storage - it should NOT be marked as explicitly set (no value in storage)
        app.update(|ctx| {
            let setting_from_storage = SimpleSetting::new_from_storage(ctx);
            assert!(!setting_from_storage.is_value_explicitly_set());
            assert_eq!(
                *setting_from_storage.value(),
                SimpleSetting::default_value()
            );
        });
    });
}

#[test]
fn test_toml_path_returns_some_for_public_setting() {
    assert_eq!(SimpleSetting::toml_path(), Some("test.simple_setting"));
}

#[test]
fn test_toml_path_returns_none_for_private_setting() {
    assert_eq!(KeyOverrideSetting::toml_path(), None);
}

#[test]
fn test_storage_key_is_struct_name_not_toml_path() {
    // storage_key() returns the struct name, not the toml_path segment.
    // The TOML backend handles snake_case conversion separately.
    assert_eq!(SimpleSetting::storage_key(), "SimpleSetting");
}

#[test]
fn test_hierarchy_derived_from_toml_path() {
    // SimpleSetting has toml_path "test.simple_setting" → hierarchy Some("test")
    assert_eq!(SimpleSetting::hierarchy(), Some("test"));
    // HierarchyFlag has toml_path "test_section.hierarchy_flag" → hierarchy Some("test_section")
    assert_eq!(HierarchyFlag::hierarchy(), Some("test_section"));
}

#[test]
fn test_hierarchy_returns_none_for_private_setting() {
    assert_eq!(KeyOverrideSetting::hierarchy(), None);
}

#[test]
fn test_private_setting_storage_key_is_explicit_override() {
    // KeyOverrideSetting has storage_key: "KeyIsOverridden" (no toml_path)
    assert_eq!(KeyOverrideSetting::storage_key(), "KeyIsOverridden");
}

#[test]
fn test_load_value_updates_value_without_persisting() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Load a non-default value.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings
                    .simple_setting
                    .load_value(true, true, ctx)
                    .unwrap();
            });
        });

        // In-memory value should be updated.
        app.read(|ctx| {
            assert!(*TestSettings::as_ref(ctx).simple_setting.value());
        });

        // Storage should still be empty — load_value must not persist.
        app.read(|ctx| {
            let prefs = SimpleSetting::preferences_for_setting(ctx);
            let stored = prefs.read_value(SimpleSetting::storage_key()).unwrap();
            assert!(stored.is_none(), "load_value should not write to storage");
        });
    });
}

#[test]
fn test_load_value_emits_event_on_change() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        let handle = app.add_model(EventListener::new);

        app.read(|ctx| {
            assert!(!handle.as_ref(ctx).got_event);
        });

        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings
                    .simple_setting
                    .load_value(true, true, ctx)
                    .unwrap();
            });
        });

        app.read(|ctx| {
            assert!(
                handle.as_ref(ctx).got_event,
                "load_value should emit event when value changes"
            );
        });
    });
}

#[test]
fn test_load_value_skips_event_when_unchanged() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        let handle = app.add_model(EventListener::new);

        // Load the same value as the default (false) with explicitly_set=false
        // (matching the initial state).
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings
                    .simple_setting
                    .load_value(false, false, ctx)
                    .unwrap();
            });
        });

        app.read(|ctx| {
            assert!(
                !handle.as_ref(ctx).got_event,
                "load_value should not emit event when value and explicitly_set are unchanged"
            );
        });
    });
}

#[test]
fn test_load_value_updates_explicitly_set_flag() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Initially not explicitly set.
        app.read(|ctx| {
            assert!(
                !TestSettings::as_ref(ctx)
                    .simple_setting
                    .is_value_explicitly_set()
            );
        });

        // Load with explicitly_set=true.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings
                    .simple_setting
                    .load_value(true, true, ctx)
                    .unwrap();
            });
        });

        app.read(|ctx| {
            assert!(
                TestSettings::as_ref(ctx)
                    .simple_setting
                    .is_value_explicitly_set()
            );
        });
    });
}

#[test]
fn test_load_value_resets_explicitly_set_flag() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // First, explicitly set a value.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings.simple_setting.set_value(true, ctx).unwrap();
            });
        });

        app.read(|ctx| {
            assert!(
                TestSettings::as_ref(ctx)
                    .simple_setting
                    .is_value_explicitly_set()
            );
        });

        // Load back the default with explicitly_set=false (simulates key removed from file).
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings
                    .simple_setting
                    .load_value(SimpleSetting::default_value(), false, ctx)
                    .unwrap();
            });
        });

        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(!settings.simple_setting.is_value_explicitly_set());
            assert_eq!(
                *settings.simple_setting.value(),
                SimpleSetting::default_value()
            );
        });
    });
}

#[test]
fn test_explicit_value_tracking_cloud_sync() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        // Register our TestSettings settings group with the app.
        TestSettings::register(&mut app);

        // Initially not explicitly set
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(!settings.simple_setting.is_value_explicitly_set());
        });

        // Set value from cloud sync
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings
                    .simple_setting
                    .set_value_from_cloud_sync(true, ctx);
            });
        });

        // Should now be marked as explicitly set
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(settings.simple_setting.is_value_explicitly_set());
            assert!(*settings.simple_setting.value());
        });
    });
}

mod file_transform_tests {
    use settings_value::SettingsValue;

    #[test]
    fn test_vec_u32_uses_serde_passthrough() {
        // Vec<u32> uses the default SettingsValue (serde passthrough)
        let value = vec![10u32, 20, 30];
        let file_val = value.to_file_value();
        assert_eq!(file_val, serde_json::json!([10, 20, 30]));
        let back = Vec::<u32>::from_file_value(&file_val).unwrap();
        assert_eq!(back, value);
    }

    #[test]
    fn test_bool_settings_value() {
        assert_eq!(true.to_file_value(), serde_json::json!(true));
        assert_eq!(
            bool::from_file_value(&serde_json::json!(false)),
            Some(false)
        );
    }
}

// ---------------------------------------------------------------------------
// Private / public settings split tests
// ---------------------------------------------------------------------------

#[test]
fn test_is_private_returns_false_for_public_setting() {
    assert!(!SimpleSetting::is_private());
}

#[test]
fn test_is_private_returns_true_for_private_setting() {
    assert!(KeyOverrideSetting::is_private());
}

#[test]
fn test_public_setting_writes_to_public_prefs_when_flag_enabled() {
    let _guard = warp_features::FeatureFlag::SettingsFile.override_enabled(true);
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Set the public setting to a non-default value.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings.simple_setting.set_value(true, ctx).unwrap();
            });
        });

        // The value should be in the public (Model) backend.
        app.read(|ctx| {
            let public = <crate::PublicPreferences as SingletonEntity>::as_ref(ctx);
            let stored = public
                .as_preferences()
                .read_value(SimpleSetting::storage_key())
                .unwrap();
            assert!(stored.is_some(), "public setting should be in public prefs");
        });

        // The value should NOT be in the private backend.
        app.read(|ctx| {
            let private = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx);
            let stored = private.0.read_value(SimpleSetting::storage_key()).unwrap();
            assert!(
                stored.is_none(),
                "public setting should not be in private prefs"
            );
        });
    });
}

#[test]
fn test_private_setting_writes_to_private_prefs_when_flag_enabled() {
    let _guard = warp_features::FeatureFlag::SettingsFile.override_enabled(true);
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Set the private setting to a non-default value.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings
                    .key_override_setting
                    .set_value(false, ctx)
                    .unwrap();
            });
        });

        // The value should be in the private backend.
        app.read(|ctx| {
            let private = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx);
            let stored = private
                .0
                .read_value(KeyOverrideSetting::storage_key())
                .unwrap();
            assert!(
                stored.is_some(),
                "private setting should be in private prefs"
            );
        });

        // The value should NOT be in the public backend.
        app.read(|ctx| {
            let public = <crate::PublicPreferences as SingletonEntity>::as_ref(ctx);
            let stored = public
                .as_preferences()
                .read_value(KeyOverrideSetting::storage_key())
                .unwrap();
            assert!(
                stored.is_none(),
                "private setting should not be in public prefs"
            );
        });
    });
}

#[test]
fn test_new_from_storage_reads_from_correct_backend_when_flag_enabled() {
    let _guard = warp_features::FeatureFlag::SettingsFile.override_enabled(true);
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Write a value directly to the public backend for the public setting.
        app.update(|ctx| {
            let public = <crate::PublicPreferences as SingletonEntity>::as_ref(ctx);
            public
                .as_preferences()
                .write_value(SimpleSetting::storage_key(), "true".to_string())
                .unwrap();
        });

        // Write a value directly to the private backend for the private setting.
        app.update(|ctx| {
            let private = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx);
            private
                .0
                .write_value(KeyOverrideSetting::storage_key(), "false".to_string())
                .unwrap();
        });

        // new_from_storage should read from the correct backends.
        app.update(|ctx| {
            let public_setting = SimpleSetting::new_from_storage(ctx);
            assert!(*public_setting.value());

            let private_setting = KeyOverrideSetting::new_from_storage(ctx);
            assert!(!*private_setting.value());
        });
    });
}

#[test]
fn test_clear_value_clears_from_correct_backend() {
    let _guard = warp_features::FeatureFlag::SettingsFile.override_enabled(true);
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Set both settings.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings.simple_setting.set_value(true, ctx).unwrap();
                test_settings
                    .key_override_setting
                    .set_value(false, ctx)
                    .unwrap();
            });
        });

        // Clear both settings.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings.simple_setting.clear_value(ctx).unwrap();
                test_settings.key_override_setting.clear_value(ctx).unwrap();
            });
        });

        // Public backend should no longer have the public setting.
        app.read(|ctx| {
            let public = <crate::PublicPreferences as SingletonEntity>::as_ref(ctx);
            assert!(
                public
                    .as_preferences()
                    .read_value(SimpleSetting::storage_key())
                    .unwrap()
                    .is_none(),
                "cleared public setting should not be in public prefs"
            );
        });

        // Private backend should no longer have the private setting.
        app.read(|ctx| {
            let private = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx);
            assert!(
                private
                    .0
                    .read_value(KeyOverrideSetting::storage_key())
                    .unwrap()
                    .is_none(),
                "cleared private setting should not be in private prefs"
            );
        });
    });
}

#[test]
fn test_public_setting_uses_private_prefs_when_flag_disabled() {
    let _guard = warp_features::FeatureFlag::SettingsFile.override_enabled(false);
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Set the public setting.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings.simple_setting.set_value(true, ctx).unwrap();
            });
        });

        // With the flag disabled, public settings fall back to the private backend.
        app.read(|ctx| {
            let private = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx);
            let stored = private.0.read_value(SimpleSetting::storage_key()).unwrap();
            assert!(
                stored.is_some(),
                "public setting should be in private prefs when flag is disabled"
            );
        });

        // The public backend should NOT have it.
        app.read(|ctx| {
            let public = <crate::PublicPreferences as SingletonEntity>::as_ref(ctx);
            let stored = public
                .as_preferences()
                .read_value(SimpleSetting::storage_key())
                .unwrap();
            assert!(
                stored.is_none(),
                "public setting should not be in public prefs when flag is disabled"
            );
        });
    });
}

#[test]
fn test_private_setting_uses_private_prefs_when_flag_disabled() {
    let _guard = warp_features::FeatureFlag::SettingsFile.override_enabled(false);
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Set the private setting.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings
                    .key_override_setting
                    .set_value(false, ctx)
                    .unwrap();
            });
        });

        // Private settings always go to the private backend.
        app.read(|ctx| {
            let private = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx);
            let stored = private
                .0
                .read_value(KeyOverrideSetting::storage_key())
                .unwrap();
            assert!(
                stored.is_some(),
                "private setting should be in private prefs when flag is disabled"
            );
        });
    });
}

#[test]
fn test_new_from_storage_reads_from_private_backend_when_flag_disabled() {
    let _guard = warp_features::FeatureFlag::SettingsFile.override_enabled(false);
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Write values directly to the private backend for both settings.
        app.update(|ctx| {
            let private = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx);
            private
                .0
                .write_value(SimpleSetting::storage_key(), "true".to_string())
                .unwrap();
            private
                .0
                .write_value(KeyOverrideSetting::storage_key(), "false".to_string())
                .unwrap();
        });

        // Both settings should read from the private backend.
        app.update(|ctx| {
            let public_setting = SimpleSetting::new_from_storage(ctx);
            assert!(
                *public_setting.value(),
                "public setting should read from private prefs when flag is disabled"
            );

            let private_setting = KeyOverrideSetting::new_from_storage(ctx);
            assert!(
                !*private_setting.value(),
                "private setting should read from private prefs when flag is disabled"
            );
        });
    });
}

// ---------------------------------------------------------------------------
// SettingsManager private/public routing tests
// ---------------------------------------------------------------------------

#[test]
fn test_manager_is_private_for_storage_key() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        app.read(|ctx| {
            let manager = SettingsManager::as_ref(ctx);
            assert!(
                !manager.is_private_for_storage_key("SimpleSetting"),
                "SimpleSetting should not be private"
            );
            assert!(
                manager.is_private_for_storage_key("KeyIsOverridden"),
                "KeyIsOverridden should be private"
            );
            assert!(
                !manager.is_private_for_storage_key("UnknownKey"),
                "unknown key should default to not private"
            );
        });
    });
}

#[test]
fn test_manager_default_values_for_settings_file_excludes_private() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        app.read(|ctx| {
            let manager = SettingsManager::as_ref(ctx);
            let keys: Vec<&str> = manager
                .default_values_for_settings_file()
                .map(|(key, _, _, _)| key)
                .collect();
            assert!(
                keys.contains(&"simple_setting"),
                "public setting should appear in settings file defaults"
            );
            assert!(
                keys.contains(&"hierarchy_flag"),
                "public hierarchy setting should appear in settings file defaults"
            );
            assert!(
                !keys.contains(&"KeyIsOverridden"),
                "private setting should not appear in settings file defaults"
            );
        });
    });
}

#[test]
fn test_manager_read_local_setting_value_routes_when_flag_enabled() {
    let _guard = warp_features::FeatureFlag::SettingsFile.override_enabled(true);
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Write directly to the correct backends.
        app.update(|ctx| {
            let public = <crate::PublicPreferences as SingletonEntity>::as_ref(ctx);
            public
                .as_preferences()
                .write_value("SimpleSetting", "true".to_string())
                .unwrap();

            let private = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx);
            private
                .0
                .write_value("KeyIsOverridden", "false".to_string())
                .unwrap();
        });

        app.read(|ctx| {
            let manager = SettingsManager::as_ref(ctx);

            let public_val = manager
                .read_local_setting_value("SimpleSetting", ctx)
                .unwrap();
            assert_eq!(
                public_val,
                Some("true".to_string()),
                "manager should read public setting from public backend"
            );

            let private_val = manager
                .read_local_setting_value("KeyIsOverridden", ctx)
                .unwrap();
            assert_eq!(
                private_val,
                Some("false".to_string()),
                "manager should read private setting from private backend"
            );
        });
    });
}

#[test]
fn test_manager_read_local_setting_value_falls_back_when_flag_disabled() {
    let _guard = warp_features::FeatureFlag::SettingsFile.override_enabled(false);
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Write both values to the private backend.
        app.update(|ctx| {
            let private = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx);
            private
                .0
                .write_value("SimpleSetting", "true".to_string())
                .unwrap();
            private
                .0
                .write_value("KeyIsOverridden", "false".to_string())
                .unwrap();
        });

        app.read(|ctx| {
            let manager = SettingsManager::as_ref(ctx);

            let public_val = manager
                .read_local_setting_value("SimpleSetting", ctx)
                .unwrap();
            assert_eq!(
                public_val,
                Some("true".to_string()),
                "public setting should fall back to private backend"
            );

            let private_val = manager
                .read_local_setting_value("KeyIsOverridden", ctx)
                .unwrap();
            assert_eq!(
                private_val,
                Some("false".to_string()),
                "private setting should read from private backend"
            );
        });
    });
}

/// Regression test for the settings sync disappearing on restart bug: when
/// the TOML settings file is enabled, `read_local_setting_value` must
/// forward the setting's hierarchy, otherwise values stored under a
/// section like `[account]` are invisible to the SettingsManager and the
/// cloud preferences syncer clobbers them with stale cloud state.
#[test]
fn test_manager_read_local_setting_value_respects_hierarchy_with_settings_file() {
    use warpui_extras::user_preferences::toml_backed::TomlBackedUserPreferences;

    let _guard = warp_features::FeatureFlag::SettingsFile.override_enabled(true);
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("settings.toml");

    warpui::App::test((), |mut app| async move {
        // Use the TOML-backed store for public preferences so the hierarchy
        // routing actually matters; in-memory preferences ignore hierarchy
        // entirely and would hide this bug.
        let file_path_for_public = file_path.clone();
        app.add_singleton_model(move |_| {
            let (prefs, _) = TomlBackedUserPreferences::new(file_path_for_public);
            crate::PublicPreferences::new(Box::new(prefs))
        });
        app.add_singleton_model(|_| {
            crate::PrivatePreferences::new(Box::<
                warpui_extras::user_preferences::in_memory::InMemoryPreferences,
            >::default())
        });
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Toggle a public, hierarchy-scoped setting via the normal write
        // path. `set_value` writes to the TOML under `[test_section]`.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings.hierarchy_flag.set_value(true, ctx).unwrap();
            });
        });

        // The SettingsManager should see the value we just wrote. Without
        // the fix, this returned None because the read path looked at the
        // root table instead of `[test_section]`.
        app.read(|ctx| {
            let manager = SettingsManager::as_ref(ctx);
            let value = manager
                .read_local_setting_value("HierarchyFlag", ctx)
                .unwrap();
            assert_eq!(
                value,
                Some("true".to_string()),
                "SettingsManager must forward hierarchy when reading from \
                 the TOML-backed store"
            );
        });
    });
}
