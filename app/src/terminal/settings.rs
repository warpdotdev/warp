use serde::{Deserialize, Serialize};
use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};
use warp_core::features::FeatureFlag;
use warpui::units::Pixels;
use warpui::{AppContext, SingletonEntity};

use crate::settings::{AISettings, InputSettings, TerminalSpacing};

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Controls whether terminal programs can access the system clipboard via OSC 52 escape sequences.",
    rename_all = "snake_case"
)]
pub enum Osc52ClipboardAccess {
    #[default]
    #[schemars(description = "Deny all OSC 52 clipboard access.")]
    Deny,
    #[schemars(description = "Allow terminal programs to write to the clipboard, but not read.")]
    WriteOnly,
    #[schemars(description = "Allow terminal programs to both read and write the clipboard.")]
    ReadWrite,
}

impl Osc52ClipboardAccess {
    pub fn allows_write(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }

    pub fn allows_read(self) -> bool {
        matches!(self, Self::ReadWrite)
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Terminal block spacing.", rename_all = "snake_case")]
pub enum SpacingMode {
    #[default]
    #[schemars(description = "Normal")]
    Normal,
    #[schemars(description = "Compact")]
    Compact,
}

impl SpacingMode {
    pub fn other_mode(&self) -> SpacingMode {
        match *self {
            SpacingMode::Normal => SpacingMode::Compact,
            SpacingMode::Compact => SpacingMode::Normal,
        }
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "How padding is applied in full-screen terminal apps.",
    rename_all = "snake_case"
)]
pub enum AltScreenPaddingMode {
    #[schemars(description = "Use the same padding as the block list.")]
    MatchBlocklist,
    #[schemars(description = "Use a custom uniform padding value.")]
    Custom { uniform_padding: Pixels },
}

impl Default for AltScreenPaddingMode {
    fn default() -> Self {
        Self::Custom {
            uniform_padding: Pixels::zero(),
        }
    }
}

impl AltScreenPaddingMode {
    pub fn toggled(&self) -> Self {
        match self {
            Self::MatchBlocklist => Self::Custom {
                uniform_padding: Pixels::zero(),
            },
            Self::Custom { .. } => Self::MatchBlocklist,
        }
    }

    pub fn telemetry_string(&self) -> String {
        match self {
            Self::MatchBlocklist => "MatchBlocklist",
            Self::Custom { .. } => "Custom",
        }
        .to_string()
    }
}

define_settings_group!(TerminalSettings, settings: [
    use_audible_bell: UseAudibleBell {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP, /* Audible bell is not supported on web */
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "terminal.use_audible_bell",
        description: "Whether to play an audible bell sound on terminal bell events.",
    },
    spacing_mode: Spacing {
        type: SpacingMode,
        default: SpacingMode::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.spacing",
        description: "Controls the spacing between terminal blocks.",
    }
    maximum_grid_size: MaximumGridSize {
        type: usize,
        default: 50_000,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "terminal.maximum_grid_size",
        description: "The maximum number of rows in the terminal grid.",
    },
    alt_screen_padding: AltScreenPadding {
        type: AltScreenPaddingMode,
        default: AltScreenPaddingMode::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.full_screen_apps.alt_screen_padding",
        max_table_depth: 0,
        description: "Controls padding around full-screen terminal applications.",
    },
    // This field should not be referenced directly to check zero state block visibility -- use
    // the `should_show_zero_state_block()` getter, which also considers global AI enablement.
    show_terminal_zero_state_block: ShowTerminalZeroStateBlock {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "terminal.show_terminal_zero_state_block",
        description: "Whether to show the AI zero-state block in new terminal sessions.",
    },
    osc52_clipboard_access: Osc52ClipboardAccessSetting {
        type: Osc52ClipboardAccess,
        default: Osc52ClipboardAccess::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "terminal.osc52_clipboard_access",
        description: "Controls whether terminal programs can access the system clipboard via OSC 52 escape sequences. Options: deny (default), write_only, read_write.",
    },
    // Opt-in toggle for running terminal find on a background thread. Only consulted on
    // channels where `FeatureFlag::AsyncFind` is off; channels with the flag on force the
    // feature on and hide this toggle. See `is_async_find_enabled` for the composite check.
    async_find_enabled: AsyncFindEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "experimental.async_find_enabled",
        description: "Use an improved implementation of find to keep the UI responsive while searching for matches on large outputs.",
    },
]);

impl TerminalSettings {
    /// Spacing for the terminal blocks.
    pub fn terminal_spacing(&self, line_height_ratio: f32, ctx: &AppContext) -> TerminalSpacing {
        match *self.spacing_mode {
            SpacingMode::Normal => TerminalSpacing::normal(line_height_ratio, ctx),
            SpacingMode::Compact => TerminalSpacing::compact(line_height_ratio, ctx),
        }
    }

    /// Whether the terminal zero state block should be shown.
    /// Checks both the user setting and the global AI enablement.
    pub fn should_show_zero_state_block(&self, ctx: &AppContext) -> bool {
        *self.show_terminal_zero_state_block && AISettings::as_ref(ctx).is_any_ai_enabled(ctx)
    }

    /// Whether asynchronous terminal find should be used. On channels where
    /// `FeatureFlag::AsyncFind` is on, the feature is force-enabled (no toggle shown).
    /// On other channels, users opt in via the `async_find_enabled` setting.
    pub fn is_async_find_enabled(&self) -> bool {
        FeatureFlag::AsyncFind.is_enabled() || *self.async_find_enabled
    }

    /// Spacing for the input box.
    pub fn terminal_input_spacing(
        &self,
        line_height_ratio: f32,
        ctx: &AppContext,
    ) -> TerminalSpacing {
        let should_force_normal_spacing =
            InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx);
        if should_force_normal_spacing {
            return TerminalSpacing::normal(line_height_ratio, ctx);
        }
        match *self.spacing_mode {
            SpacingMode::Normal => TerminalSpacing::normal(line_height_ratio, ctx),
            SpacingMode::Compact => TerminalSpacing::compact(line_height_ratio, ctx),
        }
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
