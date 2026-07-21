use warp::tui_export::{InputConfig, InputType};

use super::{
    AI_LOCKED_CONFIG, AI_UNLOCKED_CONFIG, SHELL_LOCKED_CONFIG, agent_config_for_autodetection,
    config_on_autodetection_setting_changed,
};

#[test]
fn agent_default_tracks_autodetection_setting() {
    assert_eq!(agent_config_for_autodetection(true), AI_UNLOCKED_CONFIG);
    assert_eq!(agent_config_for_autodetection(false), AI_LOCKED_CONFIG);
}

#[test]
fn setting_change_returns_to_agent_default() {
    let detected_shell = InputConfig {
        input_type: InputType::Shell,
        is_locked: false,
    };
    assert_eq!(
        config_on_autodetection_setting_changed(detected_shell, false),
        Some(AI_LOCKED_CONFIG)
    );
    assert_eq!(
        config_on_autodetection_setting_changed(AI_LOCKED_CONFIG, true),
        Some(AI_UNLOCKED_CONFIG)
    );
}

#[test]
fn setting_change_preserves_explicit_shell_override() {
    assert_eq!(
        config_on_autodetection_setting_changed(SHELL_LOCKED_CONFIG, true),
        None
    );
    assert_eq!(
        config_on_autodetection_setting_changed(SHELL_LOCKED_CONFIG, false),
        None
    );
}
