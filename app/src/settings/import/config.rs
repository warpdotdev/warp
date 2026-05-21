use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use pathfinder_color::ColorU;
use serde::Serialize;
use strum_macros::EnumIter;
use thiserror::Error;
use warp_core::ui::color::hex_color::HexColorError as UiHexColorError;
use warp_core::ui::theme::{AnsiColors, WarpTheme};
use warpui::fonts::FontInfo;
use warpui::keymap::Keystroke;
use warpui::DisplayIdx;

use super::alacritty_parser::AlacrittyConfig;
#[cfg(target_os = "macos")]
use super::iterm_parser::ITermProfile;
use super::model::TerminalType;
use crate::interval_timer::IntervalTimer;
use crate::root_view::QuakeModePinPosition;
use crate::settings::ExtraMetaKeys;
use crate::terminal::session_settings::{StartupShell, WorkingDirectoryConfig};
use crate::themes::theme_creator::pick_accent_color_from_options;
#[cfg(feature = "local_fs")]
use crate::{themes::theme_creator_body::ThemeCreatorBody, user_config};

#[derive(Debug)]
pub enum ThemeType {
    LightAndDark { light: WarpTheme, dark: WarpTheme },
    Single(WarpTheme),
}

#[derive(Clone, Debug)]
pub enum ThemeError {
    /// A hex color is malformatted (not missing).
    HexColorError(UiHexColorError),
    /// A value in the theme is missing.
    MissingValueError,
}

#[derive(Clone, Error, Debug)]
pub enum HotkeyError {
    #[error("A hotkey window opens in a way Warp does not support")]
    UnsupportedWindowType,
    #[error("There are multiple hotkeys configured")]
    MultipleHotkeys,
    #[error("No hotkey is set")]
    MissingHotkey,
}

#[derive(Debug)]
pub enum ConfigError {
    /// A general IO error when reading the file, excluding
    /// a NotFound error.
    FileIOError(std::io::Error),
    /// A file is missing.
    FileNotFoundError,
    /// A file is readable but is formatted incorrectly.
    MalformattedFileError(PathBuf),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, EnumIter, Serialize)]
pub enum SettingType {
    Theme,
    OptionAsMeta,
    MouseAndScrollReporting,
    Font,
    DefaultShell,
    WorkingDirectory,
    HotkeyMode,
    WindowSize,
    CopyOnSelect,
    Opacity,
    CursorBlinking,
}

impl SettingType {
    pub fn get_name(&self) -> &'static str {
        match self {
            SettingType::Theme => "Theme",
            SettingType::OptionAsMeta => "Option as Meta",
            SettingType::MouseAndScrollReporting => "Mouse/Scroll Reporting",
            SettingType::Font => "Font",
            SettingType::DefaultShell => "Default Shell",
            SettingType::WorkingDirectory => "Working Directory",
            SettingType::HotkeyMode => "Global hotkey",
            SettingType::WindowSize => "Window Dimensions",
            SettingType::CopyOnSelect => "Copy On Select",
            SettingType::Opacity => "Window Opacity",
            SettingType::CursorBlinking => "Cursor Blinking",
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MouseAndScrollReporting {
    pub mouse_reporting: bool,
    pub scroll_reporting: bool,
}

impl Default for MouseAndScrollReporting {
    fn default() -> Self {
        Self {
            mouse_reporting: true,
            scroll_reporting: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedFont {
    pub family: Option<String>,
    pub size: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuakeModeWindow {
    pub keystroke: Keystroke,
    pub autohide: bool,
    pub screen: Option<DisplayIdx>,
    pub pin_position: QuakeModePinPosition,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GlobalHotkey {
    Activation(Keystroke),
    QuakeMode(QuakeModeWindow),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpacitySettings {
    pub opacity: Option<u8>,
    pub blur_radius: Option<u8>,
}

#[derive(Debug)]
pub struct Config {
    pub theme: ImportableSetting<Result<ThemeType, ThemeError>>,
    pub option_as_meta: ImportableSetting<ExtraMetaKeys>,
    pub mouse_and_scroll_reporting: ImportableSetting<Option<MouseAndScrollReporting>>,
    pub font: ImportableSetting<ImportedFont>,
    pub default_shell: ImportableSetting<Option<StartupShell>>,
    pub working_directory: ImportableSetting<Option<WorkingDirectoryConfig>>,
    pub terminal_name: String,
    pub description: Option<String>,
    pub hotkey_mode: ImportableSetting<Result<GlobalHotkey, HotkeyError>>,
    pub opacity: ImportableSetting<OpacitySettings>,
    pub window_size: ImportableSetting<(Option<u16>, Option<u16>)>,
    pub copy_on_select: ImportableSetting<Option<bool>>,
    pub cursor_blinking: ImportableSetting<Option<bool>>,
}

impl Config {
    /// Creates all valid Configs for a given ParseableConfig type.
    /// Each profile (in terminals that support profiles) is mapped to a separate Config.
    pub async fn create_from_external_configs<Input: ParseableConfig>(
        fonts: Arc<Vec<FontInfo>>,
    ) -> (Result<Vec<Self>, ConfigError>, IntervalTimer) {
        let mut timer = IntervalTimer::new();
        let config = match Input::from_config_paths(&mut timer).await {
            Ok(config) => config,
            Err(err) => {
                return (Err(err), timer);
            }
        };
        let configs = config
            .into_iter()
            .map(|config| config.remove_default_values())
            .map(|config| config.parse(&fonts))
            .filter(|config| config.is_valid())
            .collect();
        timer.mark_interval_end("TERMINAL_SETTINGS_PARSED");
        (Ok(configs), timer)
    }

    pub(super) fn write_theme(&self) -> Option<ThemeType> {
        #[cfg(feature = "local_fs")]
        {
            if !self.theme.should_import {
                return None;
            }

            let Ok(theme) = self.theme.value() else {
                return None;
            };

            let dir = user_config::themes_dir();

            match theme {
                ThemeType::LightAndDark { light, dark } => {
                    let light_theme_yaml_file_name =
                        format!("{}_light_theme.yaml", self.terminal_name);
                    let light_written = ThemeCreatorBody::write_theme(
                        light,
                        dir.clone(),
                        light_theme_yaml_file_name,
                        None,
                        |_| light.clone(),
                    );

                    let dark_theme_yaml_file_name =
                        format!("{}_dark_theme.yaml", self.terminal_name);
                    let dark_written = ThemeCreatorBody::write_theme(
                        dark,
                        dir,
                        dark_theme_yaml_file_name,
                        None,
                        |_| dark.clone(),
                    );

                    if let (Some(light), Some(dark)) = (light_written, dark_written) {
                        Some(ThemeType::LightAndDark { light, dark })
                    } else {
                        None
                    }
                }
                ThemeType::Single(normal) => {
                    let theme_yaml_file_name = format!("{}_theme.yaml", self.terminal_name);
                    ThemeCreatorBody::write_theme(normal, dir, theme_yaml_file_name, None, |_| {
                        ThemeType::Single(normal.clone())
                    })
                }
            }
        }
        #[cfg(not(feature = "local_fs"))]
        {
            log::warn!("Tried to save theme without a local filesystem.");
            None
        }
    }

    /// Generates a Config from the given TerminalType.
    pub async fn create_from_terminal_type(
        terminal: TerminalType,
        fonts: Arc<Vec<FontInfo>>,
    ) -> (Result<Vec<Self>, ConfigError>, IntervalTimer) {
        match terminal {
            TerminalType::Alacritty => {
                Config::create_from_external_configs::<AlacrittyConfig>(fonts).await
            }
            #[cfg(target_os = "macos")]
            TerminalType::ITerm => {
                Config::create_from_external_configs::<ITermProfile>(fonts).await
            }
        }
    }

    /// Returns the list of [`SettingType`]s that have an importable setting associated with it.
    pub(super) fn valid_setting_types(&self) -> Vec<SettingType> {
        let mut out = vec![];
        let default_config = Config::default();
        if self.theme.value().is_ok() {
            out.push(self.theme.setting_type().clone());
        }

        if *self.option_as_meta.value() != *default_config.option_as_meta.value() {
            out.push(self.option_as_meta.setting_type().clone());
        }

        if *self.mouse_and_scroll_reporting.value()
            != *default_config.mouse_and_scroll_reporting.value()
        {
            out.push(self.mouse_and_scroll_reporting.setting_type().clone());
        }

        if *self.font.value() != *default_config.font.value() {
            out.push(self.font.setting_type().clone());
        }

        if *self.default_shell.value() != *default_config.default_shell.value() {
            out.push(self.default_shell.setting_type().clone());
        }

        if *self.working_directory.value() != *default_config.working_directory.value() {
            out.push(self.working_directory.setting_type().clone());
        }
        if *self.copy_on_select.value() != *default_config.copy_on_select.value() {
            out.push(self.copy_on_select.setting_type().clone());
        }
        if *self.cursor_blinking.value() != *default_config.cursor_blinking.value() {
            out.push(self.cursor_blinking.setting_type().clone());
        }
        if *self.window_size.value() != *default_config.window_size.value() {
            out.push(self.window_size.setting_type().clone());
        }
        if *self.opacity.value() != *default_config.opacity.value() {
            out.push(self.opacity.setting_type().clone());
        }

        if self.hotkey_mode.value().is_ok() {
            out.push(self.hotkey_mode.setting_type().clone());
        }

        out
    }

    /// Returns whether or not this Config contains valid importable settings.
    pub fn is_valid(&self) -> bool {
        !self.valid_setting_types().is_empty()
    }
}

/// Used for telemetry.
#[derive(Clone, Serialize)]
pub struct ParsedTerminalSetting {
    pub setting_type: SettingType,
    pub was_imported_by_user: bool,
}

/// A wrapper for a setting along with its display name
/// and whether or not the user has selected to import it.
#[derive(Debug)]
pub struct ImportableSetting<T> {
    pub(super) setting: T,
    setting_type: SettingType,
    pub should_import: bool,
}

impl<T> ImportableSetting<T> {
    pub fn new(setting: T, setting_type: SettingType) -> Self {
        Self {
            setting,
            setting_type,
            should_import: true,
        }
    }

    /// Returns the a reference to the inner setting.
    pub fn value(&self) -> &T {
        &self.setting
    }

    /// Returns the setting type.
    pub fn setting_type(&self) -> &SettingType {
        &self.setting_type
    }
}

impl<T: Clone> ImportableSetting<T> {
    /// Returns Some(value) if we should import this setting and None otherwise.
    pub fn importable_value(&self) -> Option<T> {
        if self.should_import {
            Some(self.setting.clone())
        } else {
            None
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: ImportableSetting::new(Err(ThemeError::MissingValueError), SettingType::Theme),
            option_as_meta: ImportableSetting::new(Default::default(), SettingType::OptionAsMeta),
            mouse_and_scroll_reporting: ImportableSetting::new(
                None,
                SettingType::MouseAndScrollReporting,
            ),
            terminal_name: "".to_string(),
            font: ImportableSetting::new(
                ImportedFont {
                    family: None,
                    size: None,
                },
                SettingType::Font,
            ),
            description: None,
            default_shell: ImportableSetting::new(None, SettingType::DefaultShell),
            working_directory: ImportableSetting::new(None, SettingType::WorkingDirectory),
            hotkey_mode: ImportableSetting::new(
                Err(HotkeyError::MissingHotkey),
                SettingType::HotkeyMode,
            ),
            window_size: ImportableSetting::new((None, None), SettingType::WindowSize),
            opacity: ImportableSetting::new(
                OpacitySettings {
                    opacity: None,
                    blur_radius: None,
                },
                SettingType::Opacity,
            ),
            copy_on_select: ImportableSetting::new(None, SettingType::CopyOnSelect),
            cursor_blinking: ImportableSetting::new(None, SettingType::CursorBlinking),
        }
    }
}

#[async_trait]
pub trait ParseableConfig: PartialEq + Sized + Send {
    /// Reads the file at the given path into the struct implementing ParseableConfig.
    async fn from_file(path: PathBuf) -> Result<Vec<Self>, ConfigError>;

    /// Creates a Warp-readable `Config`. Sets corresponding errors if values have
    /// not been configured from the default.
    fn parse(self, fonts: &[FontInfo]) -> Config;

    /// Tries to read configuration from the list of default paths.
    ///
    /// NOTE: Returns an error if
    /// none of the files are found or any of the files throw some other error.
    async fn from_config_paths(timer: &mut IntervalTimer) -> Result<Vec<Self>, ConfigError> {
        for path in Self::default_paths() {
            match Self::from_file(path).await {
                Err(ConfigError::FileNotFoundError) => continue,
                result => {
                    timer.mark_interval_end("TERMINAL_SETTINGS_READ_FROM_FILE");
                    return result;
                }
            }
        }
        timer.mark_interval_end("TERMINAL_SETTINGS_READ_FROM_FILE");
        Err(ConfigError::FileNotFoundError)
    }

    /// Returns the list of paths in which to search for configuration files.
    fn default_paths() -> Vec<PathBuf>;

    /// Strips all fields that have not been configured from the default.
    fn remove_default_values(self) -> Self;
}

pub fn calculate_accent_color(
    background: impl Into<ColorU>,
    foreground: impl Into<ColorU>,
    cursor_color: impl Into<ColorU>,
    bright: AnsiColors,
) -> ColorU {
    let cursor_color = cursor_color.into();
    let foreground = foreground.into();
    if cursor_color == foreground {
        pick_accent_color_from_options(
            &[background.into(), foreground],
            // Exclude white and black so that we don't choose either.
            &[
                bright.red.into(),
                bright.green.into(),
                bright.yellow.into(),
                bright.magenta.into(),
                bright.cyan.into(),
                bright.blue.into(),
            ],
        )
    } else {
        cursor_color
    }
}
