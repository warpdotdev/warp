//! Integration tests for the settings file error banner.
//!
//! These tests verify that the workspace shows a warning banner when
//! `settings.toml` contains errors — either the entire file is unparsable
//! or individual setting values are invalid — and that the banner clears
//! when the file is fixed.

use std::time::Duration;

use warp::features::FeatureFlag;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::workspace_view;
use warpui_core::async_assert;
use warpui_core::integration::TestStep;

use super::{Builder, new_builder};

/// Helper: returns the path to the TOML settings file.
fn toml_file_path() -> std::path::PathBuf {
    warp::settings::user_preferences_toml_file_path()
}

// ---------------------------------------------------------------------------
// Startup: whole file unparsable
// ---------------------------------------------------------------------------

/// Verifies that when `settings.toml` contains invalid TOML syntax on
/// startup, the workspace shows the settings error banner.
pub fn test_settings_error_banner_on_startup_with_invalid_toml() -> Builder {
    FeatureFlag::SettingsFile.set_enabled(true);

    new_builder()
        .with_setup(move |_utils| {
            // Write syntactically invalid TOML before the app starts.
            let path = toml_file_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("should create config dir");
            }
            std::fs::write(&path, "this is [not valid toml =").expect("should write invalid TOML");
        })
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0).add_named_assertion(
                "Settings error banner should be visible on startup",
                |app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _| {
                        async_assert!(
                            view.has_settings_file_error_banner(),
                            "Workspace should show settings error banner for unparsable TOML"
                        )
                    })
                },
            ),
        )
}

// ---------------------------------------------------------------------------
// Startup: individual invalid value
// ---------------------------------------------------------------------------

/// Verifies that when `settings.toml` contains a syntactically valid TOML
/// file but with an invalid value for a known setting, the workspace shows
/// the settings error banner.
pub fn test_settings_error_banner_on_startup_with_invalid_value() -> Builder {
    FeatureFlag::SettingsFile.set_enabled(true);

    new_builder()
        .with_setup(move |_utils| {
            // Write valid TOML with an invalid value for a bool setting.
            // `font_size` expects a float; "not_a_number" will fail deserialization.
            let path = toml_file_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("should create config dir");
            }
            std::fs::write(&path, "[appearance.text]\nfont_size = \"not_a_number\"\n")
                .expect("should write TOML with invalid value");
        })
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0).add_named_assertion(
                "Settings error banner should be visible for invalid value",
                |app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _| {
                        async_assert!(
                            view.has_settings_file_error_banner(),
                            "Workspace should show settings error banner for invalid setting value"
                        )
                    })
                },
            ),
        )
}

// ---------------------------------------------------------------------------
// Reload: whole file becomes unparsable
// ---------------------------------------------------------------------------

/// Verifies that when `settings.toml` becomes unparsable after a file
/// change, the settings error banner appears; and when the file is fixed,
/// the banner disappears.
pub fn test_settings_error_banner_on_reload_with_invalid_toml() -> Builder {
    FeatureFlag::SettingsFile.set_enabled(true);

    new_builder()
        .with_setup(move |utils| {
            // Use a short watcher delay so the reload fires quickly.
            utils.set_env("WARP_CONFIG_WATCHER_DELAY_MS", Some("10".to_string()));

            // Create a valid settings file so the watcher is already tracking
            // it. The reload tests modify this file rather than creating a new
            // one, which is more reliable for filesystem watchers.
            let path = toml_file_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("should create config dir");
            }
            std::fs::write(&path, "# valid empty settings\n")
                .expect("should write initial valid TOML");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Step 1: No banner initially.
        .with_step(
            new_step_with_default_assertions("No settings error banner initially")
                .add_named_assertion("Banner should not be visible", |app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _| {
                        async_assert!(
                            !view.has_settings_file_error_banner(),
                            "Should not show settings error banner with no errors"
                        )
                    })
                }),
        )
        // Step 2: Overwrite with invalid TOML to trigger the error banner.
        .with_step(
            TestStep::new("Write invalid TOML to settings file")
                .set_timeout(Duration::from_secs(30))
                .with_setup(|_utils| {
                    let path = toml_file_path();
                    std::fs::write(&path, "broken [toml =").expect("should write invalid TOML");
                })
                .add_named_assertion(
                    "Banner should appear after reload with invalid TOML",
                    |app, window_id| {
                        let workspace = workspace_view(app, window_id);
                        workspace.read(app, |view, _| {
                            async_assert!(
                                view.has_settings_file_error_banner(),
                                "Workspace should show settings error banner after reload"
                            )
                        })
                    },
                ),
        )
        // Step 3: Fix the file — banner should disappear.
        .with_step(
            TestStep::new("Fix the settings file")
                .set_timeout(Duration::from_secs(30))
                .with_setup(|_utils| {
                    let path = toml_file_path();
                    std::fs::write(&path, "# valid empty TOML\n").expect("should write valid TOML");
                })
                .add_named_assertion(
                    "Banner should disappear after file is fixed",
                    |app, window_id| {
                        let workspace = workspace_view(app, window_id);
                        workspace.read(app, |view, _| {
                            async_assert!(
                                !view.has_settings_file_error_banner(),
                                "Banner should clear when settings file is fixed"
                            )
                        })
                    },
                ),
        )
}

// ---------------------------------------------------------------------------
// Reload: individual value becomes invalid
// ---------------------------------------------------------------------------

/// Verifies that when an individual setting value becomes invalid after a
/// file change, the settings error banner appears.
pub fn test_settings_error_banner_on_reload_with_invalid_value() -> Builder {
    FeatureFlag::SettingsFile.set_enabled(true);

    new_builder()
        .with_setup(move |utils| {
            utils.set_env("WARP_CONFIG_WATCHER_DELAY_MS", Some("10".to_string()));

            // Create the settings file at startup so the watcher tracks it.
            let path = toml_file_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("should create config dir");
            }
            std::fs::write(&path, "[appearance.text]\nfont_size = 14.0\n")
                .expect("should write initial valid TOML");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Step 1: Verify no banner with valid settings.
        .with_step(
            new_step_with_default_assertions("No banner with valid settings").add_named_assertion(
                "No banner with valid settings",
                |app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _| {
                        async_assert!(
                            !view.has_settings_file_error_banner(),
                            "Should not show error banner with valid settings"
                        )
                    })
                },
            ),
        )
        // Step 2: Change the value to something invalid.
        .with_step(
            TestStep::new("Write invalid setting value")
                .set_timeout(Duration::from_secs(30))
                .with_setup(|_utils| {
                    let path = toml_file_path();
                    std::fs::write(&path, "[appearance.text]\nfont_size = \"not_a_number\"\n")
                        .expect("should write invalid value");
                })
                .add_named_assertion(
                    "Banner should appear for invalid value",
                    |app, window_id| {
                        let workspace = workspace_view(app, window_id);
                        workspace.read(app, |view, _| {
                            async_assert!(
                                view.has_settings_file_error_banner(),
                                "Workspace should show error banner for invalid setting value"
                            )
                        })
                    },
                ),
        )
}
