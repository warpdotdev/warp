use enum_iterator::Sequence;
use serde::{Deserialize, Serialize};
use warp_core::settings::macros::define_settings_group;
use warp_core::settings::{SupportedPlatforms, SyncToCloud};

/// The display language (i18n locale) used throughout the Warp UI.
///
/// Serialized to `settings.toml` as the BCP-47 tag (`"zh-CN"` / `"en"`) so the
/// file is human-readable and matches the tags consumed by the `i18n` crate.
/// Defaults to Simplified Chinese.
#[derive(
    Default,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    Sequence,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "The display language for the Warp UI.")]
pub enum Language {
    /// Simplified Chinese — the default UI language.
    #[default]
    #[serde(rename = "zh-CN")]
    #[schemars(rename = "zh-CN", description = "简体中文 (Simplified Chinese)")]
    ZhCn,
    #[serde(rename = "en")]
    #[schemars(rename = "en", description = "English")]
    En,
}

impl Language {
    /// The BCP-47 locale tag passed to [`i18n::set_locale`], e.g. `"zh-CN"`.
    pub fn locale_tag(self) -> &'static str {
        match self {
            Language::ZhCn => "zh-CN",
            Language::En => "en",
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.locale_tag())
    }
}

define_settings_group!(LanguageSettings, settings: [
    language: LanguageState {
        type: Language,
        default: Language::ZhCn,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        storage_key: "Language",
        toml_path: "appearance.language",
        description_key: "settings.schema.appearance.language.description",
    },
]);

impl LanguageSettings {
    /// Returns the currently selected UI language.
    pub fn language(&self) -> Language {
        *self.language
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings::Setting as _;

    #[test]
    fn defaults_to_chinese() {
        assert_eq!(*LanguageState::new(None).value(), Language::ZhCn);
    }

    #[test]
    fn locale_tags_match_i18n_catalogs() {
        assert_eq!(Language::ZhCn.locale_tag(), "zh-CN");
        assert_eq!(Language::En.locale_tag(), "en");
        // The default tag must match the i18n crate's default locale.
        assert_eq!(Language::default().locale_tag(), i18n::DEFAULT_LOCALE);
    }
}
