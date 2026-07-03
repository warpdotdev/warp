use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

define_settings_group!(ChangelogSettings, settings: [
   show_changelog_after_update: ShowChangelogAfterUpdate {
       type: bool,
       default: true,
       supported_platforms: SupportedPlatforms::ALL,
       sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
       surface: settings::SettingSurfaces::GUI,
       private: false,
       toml_path: "general.show_changelog_after_update",
       description: "Whether the changelog is shown after an update.",
   },
]);
