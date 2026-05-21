use warpui::SingletonEntity;

use crate::manager::SettingsManager;
use crate::{Setting, SupportedPlatforms, SyncToCloud};

use crate::*;

define_settings_group!(TestSettings, settings: [
    never_sync_setting: SimpleSetting {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "test.simple_setting",
    },
    global_setting: GlobalSetting {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "test.global_setting",
    },
    global_setting_no_respect: GlobalSettingNoRespect {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
        private: false,
        toml_path: "test.global_setting_no_respect",
    },
    per_platform_setting: PerPlatformSetting {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::PerPlatform(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "test.per_platform_setting",
    },
    per_platform_setting_no_respect: PerPlatformSettingNoRespect {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::PerPlatform(RespectUserSyncSetting::No),
        private: false,
        toml_path: "test.per_platform_setting_no_respect",
    },
]);

pub fn init_and_register_user_preferences(ctx: &mut AppContext) {
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

#[test]
fn test_is_setting_syncable_on_current_platform() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_user_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        // Register our TestSettings settings group with the app.
        TestSettings::register(&mut app);

        app.read(|app| {
            let settings = TestSettings::as_ref(app);
            assert!(
                !settings
                    .never_sync_setting
                    .is_setting_syncable_on_current_platform(true)
            );
            assert!(
                !settings
                    .never_sync_setting
                    .is_setting_syncable_on_current_platform(false)
            );

            assert!(
                settings
                    .global_setting
                    .is_setting_syncable_on_current_platform(true)
            );
            assert!(
                !settings
                    .global_setting
                    .is_setting_syncable_on_current_platform(false)
            );

            assert!(
                settings
                    .global_setting_no_respect
                    .is_setting_syncable_on_current_platform(true)
            );
            assert!(
                settings
                    .global_setting_no_respect
                    .is_setting_syncable_on_current_platform(false)
            );

            if cfg!(target_os = "macos") {
                assert!(
                    settings
                        .per_platform_setting
                        .is_setting_syncable_on_current_platform(true)
                );
                assert!(
                    !settings
                        .per_platform_setting
                        .is_setting_syncable_on_current_platform(false)
                );

                assert!(
                    settings
                        .per_platform_setting_no_respect
                        .is_setting_syncable_on_current_platform(true)
                );
                assert!(
                    settings
                        .per_platform_setting_no_respect
                        .is_setting_syncable_on_current_platform(false)
                );
            } else {
                assert!(
                    !settings
                        .per_platform_setting
                        .is_setting_syncable_on_current_platform(true)
                );
                assert!(
                    !settings
                        .per_platform_setting
                        .is_setting_syncable_on_current_platform(false)
                );

                assert!(
                    !settings
                        .per_platform_setting_no_respect
                        .is_setting_syncable_on_current_platform(true)
                );
                assert!(
                    !settings
                        .per_platform_setting_no_respect
                        .is_setting_syncable_on_current_platform(false)
                );
            }
        });
    });
}

mod reload_all_public_settings_tests {
    use warpui::SingletonEntity;

    use crate::manager::SettingsManager;
    use crate::{Setting, SupportedPlatforms, SyncToCloud};

    use crate::*;

    define_settings_group!(ReloadTestSettings, settings: [
        public_flag: PublicFlag {
            type: bool,
            default: false,
            supported_platforms: SupportedPlatforms::ALL,
            sync_to_cloud: SyncToCloud::Never,
            private: false,
            toml_path: "test.public_flag",
        },
        private_flag: PrivateFlag {
            type: bool,
            default: false,
            supported_platforms: SupportedPlatforms::ALL,
            sync_to_cloud: SyncToCloud::Never,
            private: true,
        },
    ]);

    fn init_prefs(ctx: &mut AppContext) {
        ctx.add_singleton_model(move |_| -> crate::PublicPreferences {
            crate::PublicPreferences::new(Box::<
                warpui_extras::user_preferences::in_memory::InMemoryPreferences,
            >::default())
        });
        ctx.add_singleton_model(move |_| -> crate::PrivatePreferences {
            crate::PrivatePreferences(Box::<
                warpui_extras::user_preferences::in_memory::InMemoryPreferences,
            >::default())
        });
    }

    /// Verifies that `reload_all_public_settings` picks up values present
    /// in the preferences backend.
    #[test]
    fn test_loads_present_keys() {
        warpui::App::test((), |mut app| async move {
            app.update(init_prefs);
            app.add_singleton_model(|_| SettingsManager::default());
            ReloadTestSettings::register(&mut app);

            // Write a non-default value directly to the public backend.
            app.update(|ctx| {
                let public =
                    <crate::PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences();
                public
                    .write_value("public_flag", "true".to_string())
                    .unwrap();
            });

            // Reload — the in-memory value should update.
            app.update(|ctx| {
                SettingsManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.reload_all_public_settings(ctx);
                });
            });

            app.read(|ctx| {
                assert!(
                    *ReloadTestSettings::as_ref(ctx).public_flag.value(),
                    "reload should load present key from preferences"
                );
            });
        });
    }

    /// Verifies that absent keys are reset to their default values during
    /// reload (the key-deletion scenario).
    #[test]
    fn test_resets_absent_keys_to_defaults() {
        warpui::App::test((), |mut app| async move {
            app.update(init_prefs);
            app.add_singleton_model(|_| SettingsManager::default());
            ReloadTestSettings::register(&mut app);

            // Set a non-default value via set_value (updates both in-memory and storage).
            app.update(|ctx| {
                ReloadTestSettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings.public_flag.set_value(true, ctx).unwrap();
                });
            });

            // Remove the key from storage (simulates the user deleting a key from
            // the settings file, then reload_from_disk picking up the deletion).
            app.update(|ctx| {
                let public =
                    <crate::PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences();
                public.remove_value("public_flag").unwrap();
            });

            // Reload — the in-memory value should reset to default.
            app.update(|ctx| {
                SettingsManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.reload_all_public_settings(ctx);
                });
            });

            app.read(|ctx| {
                assert_eq!(
                    *ReloadTestSettings::as_ref(ctx).public_flag.value(),
                    PublicFlag::default_value(),
                    "absent key should be reset to default"
                );
            });
        });
    }

    /// Verifies that reload does NOT write absent keys back to storage.
    /// This is the property that prevents the infinite watcher loop.
    #[test]
    fn test_absent_keys_are_not_written_back() {
        warpui::App::test((), |mut app| async move {
            app.update(init_prefs);
            app.add_singleton_model(|_| SettingsManager::default());
            ReloadTestSettings::register(&mut app);

            // Set a non-default value, then remove from storage.
            app.update(|ctx| {
                ReloadTestSettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings.public_flag.set_value(true, ctx).unwrap();
                });
            });
            app.update(|ctx| {
                let public =
                    <crate::PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences();
                public.remove_value("public_flag").unwrap();
            });

            // Reload.
            app.update(|ctx| {
                SettingsManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.reload_all_public_settings(ctx);
                });
            });

            // Storage should still be empty — reload must not write back.
            app.read(|ctx| {
                let public =
                    <crate::PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences();
                let stored = public.read_value("public_flag").unwrap();
                assert!(
                    stored.is_none(),
                    "reload should not write absent keys back to storage"
                );
            });
        });
    }

    /// Verifies that `reload_all_public_settings` returns the storage keys
    /// of settings that fail to deserialize (invalid value in file).
    #[test]
    fn test_reload_returns_failed_keys_for_invalid_values() {
        warpui::App::test((), |mut app| async move {
            app.update(init_prefs);
            app.add_singleton_model(|_| SettingsManager::default());
            ReloadTestSettings::register(&mut app);

            // Write an invalid value for the public bool setting.
            app.update(|ctx| {
                let public =
                    <crate::PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences();
                public
                    .write_value("public_flag", "not_a_bool".to_string())
                    .unwrap();
            });

            // Reload — should return the failed key.
            let failed_keys = app.update(|ctx| {
                SettingsManager::handle(ctx)
                    .update(ctx, |manager, ctx| manager.reload_all_public_settings(ctx))
            });

            assert_eq!(
                failed_keys,
                vec!["public_flag".to_string()],
                "reload should return the key that failed to deserialize"
            );

            // The setting should remain at its default (not crash).
            app.read(|ctx| {
                assert_eq!(
                    *ReloadTestSettings::as_ref(ctx).public_flag.value(),
                    PublicFlag::default_value(),
                    "setting should remain at default after failed reload"
                );
            });
        });
    }

    /// Verifies that `reload_all_public_settings` returns an empty vec
    /// when all values are valid.
    #[test]
    fn test_reload_returns_empty_vec_on_success() {
        warpui::App::test((), |mut app| async move {
            app.update(init_prefs);
            app.add_singleton_model(|_| SettingsManager::default());
            ReloadTestSettings::register(&mut app);

            // Write a valid value.
            app.update(|ctx| {
                let public =
                    <crate::PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences();
                public
                    .write_value("public_flag", "true".to_string())
                    .unwrap();
            });

            let failed_keys = app.update(|ctx| {
                SettingsManager::handle(ctx)
                    .update(ctx, |manager, ctx| manager.reload_all_public_settings(ctx))
            });

            assert!(
                failed_keys.is_empty(),
                "reload should return empty vec when all values are valid"
            );
        });
    }

    /// Verifies that `validate_all_public_settings` detects invalid stored
    /// values without modifying in-memory state.
    #[test]
    fn test_validate_detects_invalid_values() {
        warpui::App::test((), |mut app| async move {
            let _guard = warp_features::FeatureFlag::SettingsFile.override_enabled(true);
            app.update(init_prefs);
            app.add_singleton_model(|_| SettingsManager::default());
            ReloadTestSettings::register(&mut app);

            // Write an invalid value directly to the public preferences.
            app.update(|ctx| {
                let public =
                    <crate::PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences();
                public
                    .write_value("public_flag", "not_valid_json_bool".to_string())
                    .unwrap();
            });

            let invalid_keys =
                app.read(|ctx| SettingsManager::as_ref(ctx).validate_all_public_settings(ctx));

            assert_eq!(
                invalid_keys,
                vec!["public_flag".to_string()],
                "validate should detect the invalid key"
            );

            // In-memory value should be unchanged (validate is read-only).
            app.read(|ctx| {
                assert_eq!(
                    *ReloadTestSettings::as_ref(ctx).public_flag.value(),
                    PublicFlag::default_value(),
                    "validate should not modify in-memory state"
                );
            });
        });
    }

    /// Verifies that `validate_all_public_settings` returns empty when all
    /// stored values are valid.
    #[test]
    fn test_validate_returns_empty_when_all_valid() {
        warpui::App::test((), |mut app| async move {
            let _guard = warp_features::FeatureFlag::SettingsFile.override_enabled(true);
            app.update(init_prefs);
            app.add_singleton_model(|_| SettingsManager::default());
            ReloadTestSettings::register(&mut app);

            // Write a valid value.
            app.update(|ctx| {
                let public =
                    <crate::PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences();
                public
                    .write_value("public_flag", "true".to_string())
                    .unwrap();
            });

            let invalid_keys =
                app.read(|ctx| SettingsManager::as_ref(ctx).validate_all_public_settings(ctx));

            assert!(
                invalid_keys.is_empty(),
                "validate should return empty when all values are valid"
            );
        });
    }
}

mod write_to_preferences_tests {
    use crate::*;

    #[derive(
        Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
    )]
    pub struct StructWithOptionals {
        required_field: String,
        optional_field: Option<String>,
        nested: NestedStruct,
    }

    impl SettingsValue for StructWithOptionals {}

    impl Default for StructWithOptionals {
        fn default() -> Self {
            Self {
                required_field: "hello".to_string(),
                optional_field: None,
                nested: NestedStruct::default(),
            }
        }
    }

    #[derive(
        Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
    )]
    pub struct NestedStruct {
        width: u32,
        height: u32,
    }

    impl Default for NestedStruct {
        fn default() -> Self {
            Self {
                width: 100,
                height: 50,
            }
        }
    }

    define_settings_group!(StructTestSettings, settings: [
        struct_setting: StructSetting {
            type: StructWithOptionals,
            default: StructWithOptionals::default(),
            supported_platforms: SupportedPlatforms::ALL,
            sync_to_cloud: SyncToCloud::Never,
            private: false,
            toml_path: "test.struct_setting",
        },
    ]);

    /// Verifies that `write_to_preferences` does NOT write back when the stored
    /// JSON differs in formatting (missing null fields, different key ordering)
    /// but is semantically equal to the new value (in-memory backend).
    #[test]
    fn test_no_spurious_write_with_format_differences() {
        let prefs =
            Box::<warpui_extras::user_preferences::in_memory::InMemoryPreferences>::default();

        // Simulate what a TOML backend produces after a round-trip:
        // - null fields (optional_field) are stripped
        // - key ordering may differ (nested before required_field)
        let stored_json_without_nulls =
            r#"{"nested":{"width":100,"height":50},"required_field":"hello"}"#;
        prefs
            .write_value("StructSetting", stored_json_without_nulls.to_string())
            .unwrap();

        // The Rust value has optional_field: None, which serde_json serializes as
        // `"optional_field":null` with a different key order. The stored JSON
        // doesn't have that field at all and has different key ordering.
        let value = StructWithOptionals::default();
        let canonical_json = serde_json::to_string(&value).unwrap();
        assert_ne!(
            canonical_json, stored_json_without_nulls,
            "precondition: the JSON strings should differ"
        );

        // write_to_preferences should detect they're semantically equal and NOT write.
        let changed = StructSetting::write_to_preferences(&value, prefs.as_ref()).unwrap();
        assert!(
            !changed,
            "write_to_preferences should not report a change for semantically equal values"
        );
    }

    /// Test with HashMap fields (non-deterministic key order) and missing
    /// Option fields — reproduces the exact QuakeModeSettings scenario.
    #[test]
    fn test_no_spurious_write_with_hashmap_and_missing_options() {
        use std::collections::HashMap;
        use warpui_extras::user_preferences::toml_backed::TomlBackedUserPreferences;

        #[derive(
            Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
        )]
        pub struct QuakeLike {
            pub keybinding: Option<String>,
            pub active_pin_position: String,
            pub sizes: HashMap<String, u32>,
            pub pin_screen: Option<u32>,
            pub hide_when_unfocused: bool,
        }

        impl SettingsValue for QuakeLike {}

        impl Default for QuakeLike {
            fn default() -> Self {
                let mut sizes = HashMap::new();
                sizes.insert("top".to_string(), 30);
                sizes.insert("bottom".to_string(), 30);
                Self {
                    keybinding: None,
                    active_pin_position: "Top".to_string(),
                    sizes,
                    pin_screen: None,
                    hide_when_unfocused: true,
                }
            }
        }

        define_settings_group!(QuakeLikeGroup, settings: [
            quake_setting: QuakeLikeSetting {
                type: QuakeLike,
                default: QuakeLike::default(),
                supported_platforms: SupportedPlatforms::ALL,
                sync_to_cloud: SyncToCloud::Never,
                private: false,
                toml_path: "test.quake_like_setting",
            },
        ]);

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("settings.toml");
        let (prefs, _) = TomlBackedUserPreferences::new(file_path.clone());

        let value = QuakeLike::default();

        // First write.
        let changed = QuakeLikeSetting::write_to_preferences(&value, &prefs).unwrap();
        assert!(changed, "first write should report a change");

        // Second write of same value should NOT report a change.
        let changed_again = QuakeLikeSetting::write_to_preferences(&value, &prefs).unwrap();
        assert!(
            !changed_again,
            "second write of same value should not report a change on TOML backend"
        );
    }

    /// Same test but using the real TOML backend
    /// null-stripping and key-reordering happens.
    #[test]
    fn test_no_spurious_write_with_toml_backend() {
        use warpui_extras::user_preferences::toml_backed::TomlBackedUserPreferences;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("settings.toml");
        let (prefs, _) = TomlBackedUserPreferences::new(file_path.clone());

        let value = StructWithOptionals::default();

        // First write stores the value. The TOML backend will strip the null
        // `optional_field` and may reorder keys.
        let changed = StructSetting::write_to_preferences(&value, &prefs).unwrap();
        assert!(changed, "first write should report a change");

        // Verify the TOML file doesn't contain the null field.
        let contents = std::fs::read_to_string(&file_path).unwrap();
        assert!(
            !contents.contains("optional_field"),
            "TOML should strip null fields, but file contains: {contents}"
        );

        // Second write of the same value should NOT report a change,
        // even though the JSON serialization differs from what's stored.
        let changed_again = StructSetting::write_to_preferences(&value, &prefs).unwrap();
        assert!(
            !changed_again,
            "second write of same value should not report a change on TOML backend"
        );
    }
}
