use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud};
use warpui::{AppContext, SingletonEntity};

use crate::features::FeatureFlag;

define_settings_group!(LigatureSettings, settings: [
    ligature_rendering_enabled: LigatureRenderingEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.text.ligature_rendering_enabled",
        description: "Whether to render font ligatures in the terminal.",
    },
]);

pub fn should_use_ligature_rendering(app: &AppContext) -> bool {
    let enabled_in_settings = *LigatureSettings::as_ref(app)
        .ligature_rendering_enabled
        .value();

    enabled_in_settings && FeatureFlag::Ligatures.is_enabled()
}
