use warp_core::features::FeatureFlag;

use super::scripting_settings_enabled;

#[test]
fn scripting_settings_page_hidden_when_warp_control_cli_flag_is_disabled() {
    let _guard = FeatureFlag::WarpControlCli.override_enabled(false);

    assert!(!scripting_settings_enabled());
}

#[test]
fn scripting_settings_page_visible_when_warp_control_cli_flag_is_enabled() {
    let _guard = FeatureFlag::WarpControlCli.override_enabled(true);

    assert!(scripting_settings_enabled());
}
