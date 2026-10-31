use settings::{RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud};

use super::HideCursorWhileTyping;

#[test]
fn hide_cursor_while_typing_metadata_matches_macos_toggle_contract() {
    assert!(HideCursorWhileTyping::default_value());
    assert_eq!(
        HideCursorWhileTyping::toml_path(),
        Some("terminal.input.hide_cursor_while_typing")
    );
    assert!(!HideCursorWhileTyping::is_private());
    assert!(matches!(
        HideCursorWhileTyping::supported_platforms(),
        SupportedPlatforms::MAC
    ));
    assert_eq!(
        HideCursorWhileTyping::sync_to_cloud(),
        SyncToCloud::PerPlatform(RespectUserSyncSetting::Yes)
    );
}
