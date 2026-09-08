//! Integration tests for the private/public settings split.
//!
//! These tests verify that public settings are persisted to the TOML file
//! while private settings remain in the platform-native (JSON) store.

use std::collections::HashMap;

use settings::Setting as _;
use warp::features::FeatureFlag;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::settings::{CodeSettings, DebugSettings, FontSettings};
use warpui_core::integration::TestStep;
use warpui_core::{SingletonEntity, async_assert, async_assert_eq};

use super::{Builder, new_builder};

/// Helper: read the TOML settings file from disk and return its contents.
/// Returns an empty string if the file does not exist.
fn read_toml_file() -> String {
    let path = warp::settings::user_preferences_toml_file_path();
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Helper: read the JSON user preferences file from disk and return its contents.
/// Returns an empty string if the file does not exist.
fn read_json_prefs_file() -> String {
    let path = warp::settings::user_preferences_file_path();
    std::fs::read_to_string(path).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// test_private_public_settings_routing_with_flag_enabled
// ---------------------------------------------------------------------------

pub fn test_private_public_settings_routing_with_flag_enabled() -> Builder {
    FeatureFlag::SettingsFile.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Step 1: Set a public setting (FontSize) and a private setting
        // (IsShellDebugModeEnabled) to non-default values.
        .with_step(
            TestStep::new("Set public and private settings").with_action(|app, _, _| {
                FontSettings::handle(app).update(app, |settings, ctx| {
                    settings
                        .monospace_font_size
                        .set_value(18.0, ctx)
                        .expect("should set font size");
                });
                DebugSettings::handle(app).update(app, |settings, ctx| {
                    settings
                        .is_shell_debug_mode_enabled
                        .set_value(true, ctx)
                        .expect("should set debug mode");
                });
            }),
        )
        // Step 2: Verify TOML file contains the public setting but not the
        // private one.
        .with_step(
            new_step_with_default_assertions("Verify TOML has public, not private (round 1)")
                .add_named_assertion("FontSize in TOML", |_, _| {
                    let toml = read_toml_file();
                    async_assert!(
                        toml.contains("font_size"),
                        "TOML file should contain the updated font size setting"
                    )
                })
                .add_named_assertion("IsShellDebugModeEnabled not in TOML", |_, _| {
                    let toml = read_toml_file();
                    async_assert!(
                        !toml.contains("IsShellDebugModeEnabled")
                            && !toml.contains("is_shell_debug_mode_enabled"),
                        "TOML file should not contain the private setting"
                    )
                }),
        )
        // Step 3: Verify JSON prefs contain the private setting.
        .with_step(
            new_step_with_default_assertions("Verify JSON has private setting (round 1)")
                .add_named_assertion("IsShellDebugModeEnabled in JSON", |_, _| {
                    let json = read_json_prefs_file();
                    async_assert!(
                        json.contains("IsShellDebugModeEnabled"),
                        "JSON prefs should contain the private setting"
                    )
                }),
        )
        // Step 4: Set a second pair — public CodeAsDefaultEditor, private
        // DismissedCodeToolbeltNewFeaturePopup.
        .with_step(
            TestStep::new("Set second pair of settings").with_action(|app, _, _| {
                CodeSettings::handle(app).update(app, |settings, ctx| {
                    settings
                        .code_as_default_editor
                        .set_value(true, ctx)
                        .expect("should set code editor");
                    settings
                        .dismissed_code_toolbelt_new_feature_popup
                        .set_value(true, ctx)
                        .expect("should set dismissed popup");
                });
            }),
        )
        // Step 5: Verify second public setting is in TOML, second private is
        // in JSON.
        .with_step(
            new_step_with_default_assertions("Verify second pair routing")
                .add_named_assertion("CodeAsDefaultEditor in TOML", |_, _| {
                    let toml = read_toml_file();
                    async_assert!(
                        toml.contains("use_warp_as_default_editor"),
                        "TOML should contain CodeAsDefaultEditor"
                    )
                })
                .add_named_assertion(
                    "DismissedCodeToolbeltNewFeaturePopup not in TOML",
                    |_, _| {
                        let toml = read_toml_file();
                        async_assert!(
                            !toml.contains("DismissedCodeToolbeltNewFeaturePopup")
                                && !toml.contains("dismissed_code_toolbelt_new_feature_popup"),
                            "TOML should not contain the private popup setting"
                        )
                    },
                )
                .add_named_assertion("DismissedCodeToolbeltNewFeaturePopup in JSON", |_, _| {
                    let json = read_json_prefs_file();
                    async_assert!(
                        json.contains("DismissedCodeToolbeltNewFeaturePopup"),
                        "JSON prefs should contain the private popup setting"
                    )
                }),
        )
}

// ---------------------------------------------------------------------------
// test_private_settings_preloaded_and_not_leaked_to_toml
// ---------------------------------------------------------------------------

pub fn test_private_settings_preloaded_and_not_leaked_to_toml() -> Builder {
    FeatureFlag::SettingsFile.set_enabled(true);

    // Pre-populate private settings in the JSON prefs file (the private
    // backend for integration tests).
    let user_defaults = HashMap::from([
        ("IsShellDebugModeEnabled".to_owned(), "true".to_owned()),
        (
            "DismissedCodeToolbeltNewFeaturePopup".to_owned(),
            "true".to_owned(),
        ),
    ]);

    new_builder()
        .with_user_defaults(user_defaults)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Step 1: Verify the app loaded the pre-populated private settings.
        .with_step(
            new_step_with_default_assertions("Verify preloaded private settings")
                .add_named_assertion("IsShellDebugModeEnabled is true", |app, _| {
                    app.read(|ctx| {
                        let val = DebugSettings::as_ref(ctx)
                            .is_shell_debug_mode_enabled
                            .value();
                        async_assert_eq!(*val, true, "preloaded debug mode should be true")
                    })
                })
                .add_named_assertion("DismissedCodeToolbeltNewFeaturePopup is true", |app, _| {
                    app.read(|ctx| {
                        let val = CodeSettings::as_ref(ctx)
                            .dismissed_code_toolbelt_new_feature_popup
                            .value();
                        async_assert_eq!(*val, true, "preloaded popup dismissed should be true")
                    })
                }),
        )
        // Step 2: Write a public setting so the TOML file has content.
        .with_step(
            TestStep::new("Set a public setting to generate TOML content").with_action(
                |app, _, _| {
                    FontSettings::handle(app).update(app, |settings, ctx| {
                        settings
                            .monospace_font_size
                            .set_value(18.0, ctx)
                            .expect("should set font size");
                    });
                },
            ),
        )
        // Step 3: Verify TOML has the public setting but not the private ones.
        .with_step(
            new_step_with_default_assertions("TOML has public, not private")
                .add_named_assertion("FontSize in TOML", |_, _| {
                    let toml = read_toml_file();
                    async_assert!(
                        toml.contains("font_size"),
                        "TOML should contain the public font size setting"
                    )
                })
                .add_named_assertion("No private keys in TOML", |_, _| {
                    let toml = read_toml_file();
                    async_assert!(
                        !toml.contains("IsShellDebugModeEnabled")
                            && !toml.contains("is_shell_debug_mode_enabled")
                            && !toml.contains("DismissedCodeToolbeltNewFeaturePopup")
                            && !toml.contains("dismissed_code_toolbelt_new_feature_popup"),
                        "TOML should not contain any private setting keys"
                    )
                }),
        )
        // Step 4: Verify JSON prefs still have both private settings.
        .with_step(
            new_step_with_default_assertions("JSON has both private settings").add_named_assertion(
                "Private settings in JSON",
                |_, _| {
                    let json = read_json_prefs_file();
                    async_assert!(
                        json.contains("IsShellDebugModeEnabled")
                            && json.contains("DismissedCodeToolbeltNewFeaturePopup"),
                        "JSON prefs should contain both private settings"
                    )
                },
            ),
        )
}
