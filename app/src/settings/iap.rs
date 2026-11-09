use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};

define_settings_group!(IapRoutingSettings, settings: [
    routing_enabled: IapRoutingEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
]);

#[cfg(test)]
#[path = "iap_tests.rs"]
mod tests;
