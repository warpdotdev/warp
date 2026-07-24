//! Integration test for the settings file hot-reload pipeline.
//!
//! Verifies that changes to `settings.toml` on disk are picked up by the
//! filesystem watcher and pushed into the in-memory setting models, on every
//! platform where Warp watches `config_local_dir()`.

use std::time::Duration;

use settings::Setting as _;
use warp::features::FeatureFlag;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::settings::FontSettings;
use warpui_core::integration::TestStep;
use warpui_core::{SingletonEntity, async_assert_eq};

use super::{Builder, new_builder};

/// Helper: returns the path to the TOML settings file.
fn toml_file_path() -> std::path::PathBuf {
    warp::settings::user_preferences_toml_file_path()
}

/// Verifies the full settings hot-reload pipeline end-to-end: the filesystem
/// watcher detects a change to `settings.toml`, `reload_from_disk` runs, and
/// `reload_all_public_settings` pushes the new value into the in-memory
/// setting model.
pub fn test_settings_file_hot_reload_applies_new_values() -> Builder {
    FeatureFlag::SettingsFile.set_enabled(true);

    new_builder()
        .with_setup(move |utils| {
            // Use a short watcher delay so each reload fires quickly.
            utils.set_env("WARP_CONFIG_WATCHER_DELAY_MS", Some("10".to_string()));

            // Write an initial valid settings file so the watcher is already
            // tracking it and the app reads a known value at startup.
            let path = toml_file_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("should create config dir");
            }
            std::fs::write(&path, "[appearance.text]\nfont_size = 14.0\n")
                .expect("should write initial valid TOML");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Step 1: Confirm the initial value was loaded on startup. This
        // baseline rules out a later false positive where the "reload" just
        // returns the default value.
        .with_step(
            new_step_with_default_assertions("Initial font_size loaded from disk")
                .add_named_assertion("monospace_font_size == 14.0", |app, _| {
                    app.read(|ctx| {
                        let val = FontSettings::as_ref(ctx).monospace_font_size.value();
                        async_assert_eq!(
                            *val,
                            14.0,
                            "startup load should have set font size to 14.0"
                        )
                    })
                }),
        )
        // Step 2: Rewrite the file with a different valid value and wait for
        // the watcher to push the new value into the in-memory model.
        .with_step(
            TestStep::new("Hot reload font_size to 18.0")
                .set_timeout(Duration::from_secs(30))
                .with_setup(|_utils| {
                    let path = toml_file_path();
                    std::fs::write(&path, "[appearance.text]\nfont_size = 18.0\n")
                        .expect("should write updated font size");
                })
                .add_named_assertion("monospace_font_size == 18.0", |app, _| {
                    app.read(|ctx| {
                        let val = FontSettings::as_ref(ctx).monospace_font_size.value();
                        async_assert_eq!(
                            *val,
                            18.0,
                            "hot reload should have updated font size to 18.0"
                        )
                    })
                }),
        )
        // Step 3: Rewrite a second time to confirm the reload is repeatable
        // and not a one-shot effect tied to the initial load.
        .with_step(
            TestStep::new("Hot reload font_size to 16.0")
                .set_timeout(Duration::from_secs(30))
                .with_setup(|_utils| {
                    let path = toml_file_path();
                    std::fs::write(&path, "[appearance.text]\nfont_size = 16.0\n")
                        .expect("should write second updated font size");
                })
                .add_named_assertion("monospace_font_size == 16.0", |app, _| {
                    app.read(|ctx| {
                        let val = FontSettings::as_ref(ctx).monospace_font_size.value();
                        async_assert_eq!(
                            *val,
                            16.0,
                            "second hot reload should have updated font size to 16.0"
                        )
                    })
                }),
        )
}
