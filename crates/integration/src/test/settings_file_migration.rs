//! Integration tests for the one-time migration of public settings from the
//! platform-native store into the TOML settings file.

use std::collections::HashMap;

use settings::Setting as _;
use warp::features::FeatureFlag;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::settings::{BlockVisibilitySettings, ScrollSettings};
use warpui::integration::AssertionOutcome;
use warpui::{async_assert, async_assert_eq, SingletonEntity};

use super::{new_builder, Builder};

/// Verifies that when the `SettingsFile` feature flag is enabled and no TOML
/// file exists yet, public settings are migrated from the platform-native
/// store (a JSON file in integration tests) into the TOML settings file.
pub fn test_settings_file_migration_from_native_store() -> Builder {
    FeatureFlag::SettingsFile.set_enabled(true);

    let custom_scroll_multiplier: f32 = 7.0;

    new_builder()
        .with_user_defaults(HashMap::from([
            (
                "MouseScrollMultiplier".to_owned(),
                serde_json::to_string(&custom_scroll_multiplier)
                    .expect("scroll multiplier should serialize to JSON string"),
            ),
            (
                "ShouldShowBootstrapBlock".to_owned(),
                serde_json::to_string(&true)
                    .expect("bool should serialize to JSON string"),
            ),
        ]))
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0)
                .add_named_assertion(
                    "Scroll multiplier should have been migrated from native store",
                    move |app, _window_id| {
                        let scroll = app.read(|ctx| {
                            *ScrollSettings::as_ref(ctx).mouse_scroll_multiplier.value()
                        });
                        async_assert!(
                            (scroll - custom_scroll_multiplier).abs() < f32::EPSILON,
                            "Expected scroll multiplier to be {custom_scroll_multiplier} but got {scroll}"
                        )
                    },
                )
                .add_named_assertion(
                    "ShouldShowBootstrapBlock should have been migrated from native store",
                    move |app, _window_id| {
                        let show = app.read(|ctx| {
                            *BlockVisibilitySettings::as_ref(ctx)
                                .should_show_bootstrap_block
                                .value()
                        });
                        async_assert_eq!(
                            show,
                            true,
                            "Expected should_show_bootstrap_block to be true but got {show}"
                        )
                    },
                )
                .add_named_assertion(
                    "TOML settings file should contain the migrated settings",
                    move |_app, _window_id| {
                        let toml_path = warp::settings::user_preferences_toml_file_path();
                        let contents = match std::fs::read_to_string(&toml_path) {
                            Ok(c) => c,
                            Err(err) => {
                                return AssertionOutcome::failure(format!(
                                    "Failed to read TOML file at {toml_path:?}: {err}"
                                ));
                            }
                        };
                        async_assert!(
                            contents.contains("mouse_scroll_multiplier")
                                && contents.contains("should_show_bootstrap_block"),
                            "TOML file should contain migrated settings but got:\n{contents}"
                        )
                    },
                ),
        )
}
