mod accessibility;
pub mod ai;
mod alias_expansion;
pub mod app_icon;
pub mod app_installation_detection;
mod block_visibility;
mod changelog;
pub mod cloud_preferences;
pub mod cloud_preferences_syncer;
mod code;
mod debug;
mod editor;
mod emacs_bindings;
pub mod font;
mod gpu;
pub mod import;
mod init;
pub mod initializer;
mod input;
mod input_mode;
#[cfg(target_os = "linux")]
mod linux;
pub mod macros;
pub mod manager;
pub mod native_preference;
mod onboarding;
mod pane;
mod privacy;
mod same_line_prompt_block;
mod scroll;
mod select;
mod ssh;
mod theme;
mod vim_banner;

#[cfg(test)]
#[path = "schema_validation_tests.rs"]
mod schema_validation_tests;

pub use accessibility::*;
pub use ai::*;
pub use alias_expansion::*;
pub use block_visibility::*;
pub use changelog::*;
pub use cloud_preferences::*;
pub use code::*;
pub use debug::*;
pub use editor::*;
pub use emacs_bindings::*;
pub use font::*;
pub use gpu::*;
pub use init::*;
pub use input::*;
pub use input_mode::*;
#[cfg(target_os = "linux")]
pub use linux::*;
pub use native_preference::*;
pub use onboarding::*;
pub use pane::*;
pub use privacy::*;
pub use same_line_prompt_block::*;
pub use scroll::*;
pub use select::*;
pub use ssh::*;
pub use theme::*;
pub use vim_banner::*;
use warp_core::user_preferences::GetUserPreferences as _;

/// Describes errors encountered when loading settings from `settings.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsFileError {
    /// The entire file failed to parse as valid TOML.
    FileParseFailed(String),
    /// Individual setting values failed to deserialize. Contains the storage
    /// keys of the settings that could not be loaded.
    InvalidSettings(Vec<String>),
}

impl std::fmt::Display for SettingsFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileParseFailed(_) => {
                write!(f, "Couldn't parse due to invalid syntax")
            }
            Self::InvalidSettings(keys) => match keys.as_slice() {
                [key] => write!(f, "Invalid value for '{key}'"),
                _ => write!(f, "Invalid values for: {}", keys.join(", ")),
            },
        }
    }
}

impl SettingsFileError {
    /// Returns the user-facing `(heading, description)` pair used to present
    /// this error. Shared between the workspace-level banner
    /// (`Workspace::render_settings_error_banner`) and the settings nav rail
    /// footer (`render_settings_error_alert`) so the two UIs stay in sync.
    pub fn heading_and_description(&self) -> (String, String) {
        match self {
            Self::FileParseFailed(_) => (
                "Your settings file contains an error.".to_owned(),
                format!("{self}. Open the file to fix it."),
            ),
            Self::InvalidSettings(keys) => match keys.len() {
                1 => (
                    "Your settings file contains an error.".to_owned(),
                    format!("{self}. The default value is being used."),
                ),
                _ => (
                    "Your settings file contains errors.".to_owned(),
                    format!("{self}. Default values are being used."),
                ),
            },
        }
    }
}

use crate::{
    root_view::QuakeModePinPosition,
    terminal::{BlockListSettings, BlockPadding},
    themes::theme::{ThemeKind, WarpTheme},
    user_config::WarpConfig,
};
use lazy_static::lazy_static;
use pathfinder_geometry::{rect::RectF, vector::Vector2F};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use settings::Setting as _;
use std::{collections::HashMap, ops::Mul, path::PathBuf};
use warp_core::features::FeatureFlag;
use warpui::{
    elements::DEFAULT_UI_LINE_HEIGHT_RATIO, keymap::Keystroke, AppContext, DisplayIdx,
    SingletonEntity,
};

// The following are user preferences keys.
pub const CHANGELOG_VERSIONS: &str = "ChangelogVersions";
pub const RESTORE_SESSION: &str = "RestoreSession";
pub const INPUT_MODE: &str = "InputMode";
pub const ACTIVATION_HOTKEY_ENABLED: &str = "ActivationHotkeyEnabled";
pub const ACTIVATION_HOTKEY_KEYBINDING: &str = "ActivationHotkeyKeybinding";
pub const DISMISSED_AI_ASSISTANT_WELCOME_KEY: &str = "DismissedWarpAIWarmWelcome";

pub const TIMES_TO_SHOW_AUTOSUGGESTION_HINT: i8 = 2;
pub const QUAKE_WINDOW_AUTOHIDE_SUPPORTED: bool = cfg!(any(target_os = "macos", windows));

lazy_static! {
    pub static ref DEFAULT_QUAKE_MODE_SIZE_PERCENTAGES: HashMap<QuakeModePinPosition, SizePercentages> =
        HashMap::from_iter([
            (
                QuakeModePinPosition::Top,
                SizePercentages {
                    width: 100,
                    height: 30
                }
            ),
            (
                QuakeModePinPosition::Bottom,
                SizePercentages {
                    width: 100,
                    height: 30
                }
            ),
            (
                QuakeModePinPosition::Left,
                SizePercentages {
                    width: 40,
                    height: 100
                }
            ),
            (
                QuakeModePinPosition::Right,
                SizePercentages {
                    width: 40,
                    height: 100
                }
            )
        ]);
}

/// Keys which may be interpreted as the meta key.
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    PartialEq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Additional keys that act as the meta key.")]
pub struct ExtraMetaKeys {
    #[schemars(description = "Whether the left Alt key acts as meta.")]
    pub left_alt: bool,
    #[schemars(description = "Whether the right Alt key acts as meta.")]
    pub right_alt: bool,
}

#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    PartialEq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "What Ctrl+Tab does.", rename_all = "snake_case")]
pub enum CtrlTabBehavior {
    #[default]
    ActivatePrevNextTab,
    CycleMostRecentSession,
}

impl CtrlTabBehavior {
    pub fn as_dropdown_label(&self) -> &str {
        match self {
            Self::ActivatePrevNextTab => "Activate previous/next tab",
            Self::CycleMostRecentSession => "Cycle most recent session",
        }
    }
}

impl ExtraMetaKeys {
    pub fn toggle_left_key(&self) -> Self {
        ExtraMetaKeys {
            left_alt: !self.left_alt,
            right_alt: self.right_alt,
        }
    }

    pub fn toggle_right_key(&self) -> Self {
        ExtraMetaKeys {
            left_alt: self.left_alt,
            right_alt: !self.right_alt,
        }
    }
}

/// App-wide UI settings.
///
/// DO NOT ADD ANYTHING NEW HERE!
///
/// This struct is deprecated; all new settings should make use of the
/// macros in app/src/settings/macros.rs.
#[derive(Clone, Debug)]
pub struct Settings;

/// This enum is used to enforce a ternary option with a dropdown in the features page. We may
/// later allow users to have both quake mode and activation mode enabled simultaneously. If/when
/// that happens we'll remove this enum. These options are not modeled as a ternary option in the
/// serialized user-defaults, but as independent options.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub enum GlobalHotkeyMode {
    #[default]
    Disabled,
    /// "Quake mode" shows a dedicated window with special properties (thanks to it using an Appkit
    /// NSPanel).
    QuakeMode,
    /// "Activation hotkey" shows/hides all of the normal windows
    ActivationHotkey,
}

impl GlobalHotkeyMode {
    pub fn as_dropdown_label(&self) -> &str {
        match self {
            Self::Disabled => "Disabled",
            Self::QuakeMode => "Dedicated hotkey window",
            Self::ActivationHotkey => "Show/hide all windows",
        }
    }
}

#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Debug,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Window size as width and height percentages of the screen.")]
pub struct SizePercentages {
    #[schemars(description = "Width as a percentage of screen width (0–100).")]
    pub width: u8,
    #[schemars(description = "Height as a percentage of screen height (0–100).")]
    pub height: u8,
}

impl SizePercentages {
    pub fn width_decimal(&self) -> f32 {
        (self.width as f32 / 100.).min(1.)
    }

    pub fn height_decimal(&self) -> f32 {
        (self.height as f32 / 100.).min(1.)
    }
}

impl Mul<Vector2F> for SizePercentages {
    type Output = Vector2F;

    fn mul(self, rhs: Vector2F) -> Vector2F {
        Vector2F::new(
            self.width_decimal() * rhs.x(),
            self.height_decimal() * rhs.y(),
        )
    }
}

#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    PartialEq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Configuration for the hotkey window.")]
pub struct QuakeModeSettings {
    #[schemars(
        description = "Keyboard shortcut to toggle the hotkey window. Format: modifiers (cmd, ctrl, alt, shift, meta) and a key joined by '-', e.g. \"cmd-shift-a\" or \"alt-enter\". Bindings are case-sensitive: when shift is present, the key must be its shifted form (e.g., \"ctrl-shift-E\", not \"ctrl-shift-e\")."
    )]
    pub keybinding: Option<Keystroke>,
    #[schemars(description = "Screen edge where the hotkey window is pinned.")]
    pub active_pin_position: QuakeModePinPosition,
    #[schemars(description = "Window size percentages for each pin position.")]
    pub pin_position_to_size_percentages: HashMap<QuakeModePinPosition, SizePercentages>,
    #[schemars(description = "Display to pin the hotkey window to.")]
    pub pin_screen: Option<DisplayIdx>,
    /// Whether we should hide quake mode window when it loses focus, this could happen either when
    /// user focuses on another warp window or another app.
    #[schemars(description = "Whether to hide the hotkey window when it loses focus.")]
    pub hide_window_when_unfocused: bool,
}

impl Default for QuakeModeSettings {
    fn default() -> Self {
        Self {
            keybinding: Default::default(),
            active_pin_position: Default::default(),
            pin_position_to_size_percentages: DEFAULT_QUAKE_MODE_SIZE_PERCENTAGES.clone(),
            pin_screen: Default::default(),
            // Defaults to `true` only when it's supported on this platform.
            hide_window_when_unfocused: QUAKE_WINDOW_AUTOHIDE_SUPPORTED,
        }
    }
}

impl QuakeModeSettings {
    pub fn width_percentage(&self) -> u8 {
        self.size_percentages_for_pin_position(&self.active_pin_position)
            .width
    }

    pub fn height_percentage(&self) -> u8 {
        self.size_percentages_for_pin_position(&self.active_pin_position)
            .height
    }

    pub fn size_changed_from_default(&self) -> bool {
        self.size_percentages_for_pin_position(&self.active_pin_position)
            != *DEFAULT_QUAKE_MODE_SIZE_PERCENTAGES
                .get(&self.active_pin_position)
                .expect("Default should have every pin position")
    }

    pub fn size_percentages_for_pin_position(
        &self,
        pin_position: &QuakeModePinPosition,
    ) -> SizePercentages {
        *self
            .pin_position_to_size_percentages
            .get(pin_position)
            .unwrap_or_else(|| {
                DEFAULT_QUAKE_MODE_SIZE_PERCENTAGES
                    .get(&self.active_pin_position)
                    .expect("Default should have every pin position")
            })
    }

    /// Resolves the display bounds for quake mode (respecting the pinned screen setting)
    /// and calculates the window bounds.
    pub fn resolve_quake_mode_bounds(&self, ctx: &mut AppContext) -> RectF {
        let display_bounds = self
            .pin_screen
            .and_then(|display_idx| ctx.windows().bounds_for_display_idx(display_idx))
            .unwrap_or_else(|| ctx.windows().active_display_bounds());
        self.calculate_quake_mode_bounds_from_settings(display_bounds)
    }

    pub fn calculate_quake_mode_bounds_from_settings(&self, display_bounds: RectF) -> RectF {
        let size_percentages = self.size_percentages_for_pin_position(&self.active_pin_position);
        let quake_window_size = size_percentages * display_bounds.size();

        match self.active_pin_position {
            QuakeModePinPosition::Top => {
                // Position the frame in the center of the display on x-axis.
                let x_axis_offset =
                    display_bounds.size().x() * (1. - size_percentages.width_decimal()) / 2.;
                let quake_window_origin = Vector2F::new(
                    display_bounds.origin().x() + x_axis_offset,
                    display_bounds.origin().y(),
                );

                RectF::new(quake_window_origin, quake_window_size)
            }
            QuakeModePinPosition::Bottom => {
                // Position the frame in the center of the display on x-axis.
                let x_axis_offset =
                    display_bounds.size().x() * (1. - size_percentages.width_decimal()) / 2.;
                let quake_window_origin = Vector2F::new(
                    display_bounds.origin().x() + x_axis_offset,
                    display_bounds.lower_left().y() - quake_window_size.y(),
                );

                RectF::new(quake_window_origin, quake_window_size)
            }
            QuakeModePinPosition::Left => {
                // Position the frame in the center of the display on y-axis.
                let y_axis_offset =
                    display_bounds.size().y() * (1. - size_percentages.height_decimal()) / 2.;
                let quake_window_origin = Vector2F::new(
                    display_bounds.origin().x(),
                    display_bounds.origin().y() + y_axis_offset,
                );

                RectF::new(quake_window_origin, quake_window_size)
            }
            QuakeModePinPosition::Right => {
                // Position the frame in the center of the display on y-axis.
                let y_axis_offset =
                    display_bounds.size().y() * (1. - size_percentages.height_decimal()) / 2.;
                let quake_window_origin = Vector2F::new(
                    display_bounds.upper_right().x() - quake_window_size.x(),
                    display_bounds.origin().y() + y_axis_offset,
                );

                RectF::new(quake_window_origin, quake_window_size)
            }
        }
    }
}

/// Circumstances when FG color can be automatically changed to increase contrast with BG color
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
    description = "When to adjust foreground color to ensure readability against the background.",
    rename_all = "snake_case"
)]
pub enum EnforceMinimumContrast {
    /// Never change FG color
    Never,
    /// FG color can be changed, but only if the FG is specified with default colors
    #[default]
    OnlyNamedColors,
    /// FG color is changed regardless of how FG was specified
    Always,
}

impl Settings {
    pub fn has_changelog_been_shown(changelog_version: &str, ctx: &mut AppContext) -> bool {
        let changelog_versions = ctx
            .private_user_preferences()
            .read_value(CHANGELOG_VERSIONS)
            .unwrap_or_default();
        changelog_versions.is_some_and(|versions| -> bool {
            let res = serde_json::from_str::<Value>(&versions);
            match res {
                Ok(versions) => versions[&changelog_version].as_bool().unwrap_or(false),
                Err(e) => {
                    log::warn!("Error deserializing changlog user default {e}");
                    false
                }
            }
        })
    }

    pub fn mark_changelog_shown(changelog_version: &str, ctx: &mut AppContext) -> bool {
        ctx.private_user_preferences()
            .read_value(CHANGELOG_VERSIONS)
            .unwrap_or_default()
            .map_or(Ok(json!({})), |versions| {
                serde_json::from_str::<Value>(&versions)
            })
            .is_ok_and(|mut versions| {
                log::info!(
                    "Marking changelog {changelog_version} as shown in versions {versions:?}"
                );

                versions[&changelog_version] = Value::Bool(true);
                let _ = ctx.private_user_preferences().write_value(
                    CHANGELOG_VERSIONS,
                    serde_json::to_string(&versions).expect("changelog versions should serialize"),
                );
                true
            })
    }

    pub fn theme_for_theme_kind(theme_kind: &ThemeKind, ctx: &mut AppContext) -> WarpTheme {
        match theme_kind {
            ThemeKind::InMemory(in_memory_theme) => in_memory_theme.theme(),
            _ => WarpConfig::as_ref(ctx).theme_config().theme(theme_kind),
        }
    }
}

/// Terminal Spacing settings. BlockPadding and inline_separator_height values are measured in grid
/// cells, not pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalSpacing {
    pub block_padding: BlockPadding,
    pub prompt_to_editor_padding: f32,
    pub editor_bottom_padding: f32,
    pub block_borders_enabled: bool,
    pub overflow_offset: f32,
    pub subshell_separator_height: f32,
}

impl TerminalSpacing {
    pub fn normal(line_height_ratio: f32, ctx: &AppContext) -> Self {
        Self {
            block_padding: BlockPadding {
                padding_top: 1.1 * (DEFAULT_UI_LINE_HEIGHT_RATIO / line_height_ratio).min(1.0),
                command_padding_top: 0.19
                    * (DEFAULT_UI_LINE_HEIGHT_RATIO / line_height_ratio).min(1.0),
                middle: 0.5 * (DEFAULT_UI_LINE_HEIGHT_RATIO / line_height_ratio).min(1.0),
                bottom: 1. * (DEFAULT_UI_LINE_HEIGHT_RATIO / line_height_ratio).min(1.0),
            },
            prompt_to_editor_padding: 10.,
            editor_bottom_padding: 20.,
            block_borders_enabled: *BlockListSettings::as_ref(ctx).show_block_dividers.value()
                || !FeatureFlag::MinimalistUI.is_enabled(),
            overflow_offset: 12.,
            // Subshell separators are actually hidden in normal spacing b/c they are meant to be
            // shown inside the block padding instead.
            subshell_separator_height: 0.,
        }
    }

    pub fn compact(line_height_ratio: f32, ctx: &AppContext) -> Self {
        Self {
            block_padding: BlockPadding {
                padding_top: 0.3 * (DEFAULT_UI_LINE_HEIGHT_RATIO / line_height_ratio).min(1.0),
                command_padding_top: 0.,
                middle: 0.,
                bottom: 0.2 * (DEFAULT_UI_LINE_HEIGHT_RATIO / line_height_ratio).min(1.0),
            },
            prompt_to_editor_padding: 0.,
            editor_bottom_padding: 4.,
            block_borders_enabled: *BlockListSettings::as_ref(ctx).show_block_dividers.value()
                || !FeatureFlag::MinimalistUI.is_enabled(),
            overflow_offset: 6.,
            subshell_separator_height: 1.1,
        }
    }
}

/// The argument type for set_extra_meta_keys action.
#[derive(Clone)]
pub struct ExtraMetaKeysChangedArg {
    pub keys: ExtraMetaKeys,
}

/// Returns the path to the user preferences file.
pub fn user_preferences_file_path() -> PathBuf {
    warp_core::paths::config_local_dir().join("user_preferences.json")
}

/// Returns the path to the TOML settings file.
pub fn user_preferences_toml_file_path() -> PathBuf {
    warp_core::paths::config_local_dir().join("settings.toml")
}
