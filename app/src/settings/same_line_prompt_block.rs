use serde::{Deserialize, Serialize};
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};
use warp_core::define_settings_group;

#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(rename_all = "snake_case")]
pub enum SLPBlockState {
    /// The block has not been shown to the user.
    #[default]
    NotShown,

    // The block has been triggered/shown.
    Shown,

    /// The block should not be shown to the user e.g. if the user is NOT using PS1.
    DoNotShow,
}

// This isn't a user-visible setting, but rather a record of a
// Warp action that should be persisted the same way we would a setting.
//
// When a user has been shown the same line prompt onboarding block,
// we want to remember that they have already been shown it.
// That way, we skip displaying it in the future and prevent it from becoming
// an annoyance. We use a Setting for this, so we get the underlying infrastructure
// for free e.g. cloud-syncing.
define_settings_group!(SameLinePromptBlockSettings, settings: [
    same_line_prompt_block_state: SameLinePromptBlockState {
        type: SLPBlockState,
        default: SLPBlockState::NotShown,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
]);
