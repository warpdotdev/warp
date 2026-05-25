use settings::{define_settings_group, SupportedPlatforms, SyncToCloud};

pub use warp_localization::AppLanguage;

define_settings_group!(LanguageSettings, settings: [
    app_language: AppLanguageSetting {
        type: AppLanguage,
        default: AppLanguage::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        storage_key: "AppLanguage",
        toml_path: "appearance.interface.language",
        description: "The display language used by Warp.",
    },
]);
