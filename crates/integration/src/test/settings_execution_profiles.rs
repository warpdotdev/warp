//! Integration tests for file-backed AI execution profiles.

use std::path::PathBuf;
use std::time::Duration;

use warp::features::FeatureFlag;
use warp::integration_testing::settings::{
    active_execution_profile_id, create_and_select_execution_profile, default_execution_profile,
    execution_profile, has_multiple_execution_profiles,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::single_terminal_view_for_tab;
use warpui_core::async_assert;
use warpui_core::integration::TestStep;

use super::{Builder, new_builder};

const INITIAL_PROFILES: &str = r#"
[agents.execution_profiles.default]
name = "Disk Default"
read_files = "always_allow"
execute_commands = "always_ask"
base_model = "disk-model"
command_allowlist = ["git status"]

[agents.execution_profiles.code-review]
name = "Code Review"
apply_code_diffs = "always_allow"
read_files = "always_ask"
command_denylist = ["rm .*"]
directory_allowlist = ["/repo"]
web_search_enabled = true
"#;

const RELOADED_PROFILES: &str = r#"
[agents.execution_profiles.default]
name = "Reloaded Default"
read_files = "always_ask"

[agents.execution_profiles.reloaded]
name = "Reloaded Profile"
read_files = "always_allow"
directory_allowlist = ["/reloaded"]
"#;

fn settings_file_path() -> PathBuf {
    warp::settings::user_preferences_toml_file_path()
}

fn write_settings_file(contents: &str) {
    let path = settings_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("should create settings directory");
    }
    std::fs::write(path, contents).expect("should write settings file");
}

fn enable_file_backed_profiles() {
    FeatureFlag::SettingsFile.set_enabled(true);
    FeatureFlag::FileBackedExecutionProfiles.set_enabled(true);
}

pub fn test_execution_profiles_load_from_settings_file() -> Builder {
    enable_file_backed_profiles();

    new_builder()
        .with_setup(|_utils| write_settings_file(INITIAL_PROFILES))
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0).add_named_assertion(
                "Execution profile collection should be loaded from settings.toml",
                |app, _window_id| {
                    let default = default_execution_profile(app);
                    let review = execution_profile(app, "code-review");

                    async_assert!(
                        default.name == "Disk Default"
                            && default.read_files_always_allow
                            && default.execute_commands_always_ask
                            && default.base_model.as_deref() == Some("disk-model")
                            && default.command_allowlist == ["git status"]
                            && review.as_ref().is_some_and(|profile| {
                                profile.name == "Code Review"
                                    && profile.apply_code_diffs_always_allow
                                    && profile.directory_allowlist == [PathBuf::from("/repo")]
                                    && profile.web_search_enabled
                            })
                            && has_multiple_execution_profiles(app),
                        "Expected the execution profile model to expose the complete collection loaded from settings.toml"
                    )
                },
            ),
        )
}

pub fn test_execution_profile_model_persists_and_hot_reloads_settings_file() -> Builder {
    enable_file_backed_profiles();

    new_builder()
        .with_setup(|utils| {
            utils.set_env("WARP_CONFIG_WATCHER_DELAY_MS", Some("10".to_string()));
            write_settings_file(INITIAL_PROFILES);
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Create and select a file-backed execution profile").with_action(
                |app, window_id, step_data| {
                    let terminal_view_id = single_terminal_view_for_tab(app, window_id, 0).id();
                    let profile_id = create_and_select_execution_profile(
                        app,
                        terminal_view_id,
                        "Persisted Profile",
                        PathBuf::from("/tmp/profile-readable"),
                    );
                    step_data.insert("created_profile_id", profile_id);
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Profile mutation should persist to settings.toml")
                .add_named_assertion_with_data_from_prior_step(
                    "Execution profile model should expose the persisted profile",
                    |app, window_id, step_data| {
                        let profile_id = step_data
                            .get::<_, String>("created_profile_id")
                            .expect("created profile ID should be available");
                        let terminal_view_id =
                            single_terminal_view_for_tab(app, window_id, 0).id();
                        let profile = execution_profile(app, profile_id);

                        async_assert!(
                            profile.as_ref().is_some_and(|profile| {
                                profile.name == "Persisted Profile"
                                    && profile.read_files_always_allow
                                    && profile.directory_allowlist
                                        == [PathBuf::from("/tmp/profile-readable")]
                            }) && active_execution_profile_id(app, terminal_view_id) == *profile_id,
                            "Expected the created execution profile to be readable and active"
                        )
                    },
                )
                .add_named_assertion_with_data_from_prior_step(
                    "Generated profile key and values should be written to settings.toml",
                    |_app, _window_id, step_data| {
                        let profile_id = step_data
                            .get::<_, String>("created_profile_id")
                            .expect("created profile ID should be available");
                        let contents =
                            std::fs::read_to_string(settings_file_path()).unwrap_or_default();
                        let expected_table =
                            format!("[agents.execution_profiles.{profile_id}]");

                        async_assert!(
                            profile_id.starts_with("profile-")
                                && contents.contains(&expected_table)
                                && contents.contains("name = \"Persisted Profile\"")
                                && contents.contains("read_files = \"always_allow\"")
                                && contents.contains("\"/tmp/profile-readable\""),
                            "Expected settings.toml to contain the generated profile table and edited values, got:\n{contents}"
                        )
                    },
                ),
        )
        .with_step(
            TestStep::new("Replace execution profiles through settings.toml")
                .set_timeout(Duration::from_secs(30))
                .with_setup(|_utils| write_settings_file(RELOADED_PROFILES))
                .add_named_assertion_with_data_from_prior_step(
                    "Hot reload should replace profiles and clear a stale active selection",
                    |app, window_id, step_data| {
                        let created_profile_id = step_data
                            .get::<_, String>("created_profile_id")
                            .expect("created profile ID should be available");
                        let terminal_view_id =
                            single_terminal_view_for_tab(app, window_id, 0).id();
                        let default = default_execution_profile(app);
                        let reloaded_profile = execution_profile(app, "reloaded");

                        async_assert!(
                            execution_profile(app, created_profile_id).is_none()
                                && default.name == "Reloaded Default"
                                && reloaded_profile.as_ref().is_some_and(|profile| {
                                    profile.name == "Reloaded Profile"
                                        && profile.read_files_always_allow
                                })
                                && active_execution_profile_id(app, terminal_view_id) == "default",
                            "Expected hot reload to replace the collection and fall back to the default profile"
                        )
                    },
                ),
        )
}
