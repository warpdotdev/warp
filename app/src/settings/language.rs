use serde::{Deserialize, Serialize};
use settings::{
    macros::define_settings_group, RespectUserSyncSetting, SupportedPlatforms, SyncToCloud,
};

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Deserialize,
    Serialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[serde(rename_all = "snake_case")]
#[schemars(
    description = "The display language used by Warp's user interface.",
    rename_all = "snake_case"
)]
pub enum DisplayLanguage {
    #[default]
    System,
    English,
    ChineseSimplified,
}

impl DisplayLanguage {
    pub const ALL: [Self; 3] = [Self::System, Self::English, Self::ChineseSimplified];
}

define_settings_group!(LanguageSettings, settings: [
    display_language: DisplayLanguageSetting {
        type: DisplayLanguage,
        default: DisplayLanguage::System,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        storage_key: "DisplayLanguage",
        toml_path: "appearance.language",
        description: "The display language used by Warp's user interface.",
    },
]);
