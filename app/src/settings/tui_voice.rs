use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};
#[cfg(feature = "tui")]
use warpui_core::platform::keyboard::KeyCode;

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[serde(rename_all = "snake_case")]
#[schemars(rename_all = "snake_case")]
pub enum TuiVoiceInputHoldKey {
    #[default]
    None,
    AltLeft,
    AltRight,
    ControlLeft,
    ControlRight,
    SuperLeft,
    SuperRight,
    ShiftLeft,
    ShiftRight,
}

#[cfg(feature = "tui")]
impl From<TuiVoiceInputHoldKey> for Option<KeyCode> {
    fn from(key: TuiVoiceInputHoldKey) -> Self {
        match key {
            TuiVoiceInputHoldKey::None => None,
            TuiVoiceInputHoldKey::AltLeft => Some(KeyCode::AltLeft),
            TuiVoiceInputHoldKey::AltRight => Some(KeyCode::AltRight),
            TuiVoiceInputHoldKey::ControlLeft => Some(KeyCode::ControlLeft),
            TuiVoiceInputHoldKey::ControlRight => Some(KeyCode::ControlRight),
            TuiVoiceInputHoldKey::SuperLeft => Some(KeyCode::SuperLeft),
            TuiVoiceInputHoldKey::SuperRight => Some(KeyCode::SuperRight),
            TuiVoiceInputHoldKey::ShiftLeft => Some(KeyCode::ShiftLeft),
            TuiVoiceInputHoldKey::ShiftRight => Some(KeyCode::ShiftRight),
        }
    }
}

define_settings_group!(TuiVoiceSettings, settings: [
    voice_input_hold_key: TuiVoiceInputHoldKeySetting {
        type: TuiVoiceInputHoldKey,
        default: TuiVoiceInputHoldKey::default(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::TUI,
        private: false,
        toml_path: "agents.voice.voice_input_hold_key",
        description: "The modifier key held to record voice input in Warp Agent CLI. Configuring a modifier enables enhanced terminal key reporting, which may interfere with AltGr, dead-key composition, some international keyboard layouts, and shortcuts that use punctuation keys. Ctrl+S remains available without a modifier. Defaults to none. Super may be unavailable in some terminals.",
    },
]);

#[cfg(test)]
#[path = "tui_voice_tests.rs"]
mod tests;
