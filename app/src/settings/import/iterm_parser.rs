use std::path::PathBuf;

use async_trait::async_trait;
use bitflags::bitflags;
use itertools::Itertools;
use palette::Srgba;
use pathfinder_color::ColorU;
use plist::{Dictionary, Value};
use warp_core::ui::theme::{AnsiColors, TerminalColors, WarpTheme};
use warpui::fonts::FontInfo;
use warpui::keymap::Keystroke;
use warpui::platform::mac::utils::unicode_char_to_key;
use warpui::DisplayIdx;

use super::config::{
    calculate_accent_color, Config, ConfigError, GlobalHotkey, ImportableSetting, ImportedFont,
    MouseAndScrollReporting, OpacitySettings, ParseableConfig, QuakeModeWindow, SettingType,
    ThemeError, ThemeType,
};
use crate::root_view::QuakeModePinPosition;
use crate::settings::import::config::HotkeyError;
use crate::settings::{ExtraMetaKeys, DEFAULT_MONOSPACE_FONT_NAME, DEFAULT_MONOSPACE_FONT_SIZE};
use crate::terminal::local_tty::shell::is_valid_path_or_command_for_supported_shell;
use crate::terminal::session_settings::{
    StartupShell, WorkingDirectoryConfig, WorkingDirectoryMode, WorkingDirectoryPerSourceConfig,
};

extern crate plist;

const ITERM_DEFAULT_MONOSPACE_FONT_SIZE: &str = "12";
const ITERM_DEFAULT_MONOSPACE_FONT_FAMILY: &str = "Monaco";

const WARP_DEFAULT_WORKING_DIRECTORY: ITermWorkingDirectoryStrategy =
    ITermWorkingDirectoryStrategy::Simple(ITermWorkingDirectory::ReuseLast);

const PIN_TOP: i64 = 2;
const PIN_BOTTOM: i64 = 5;
const PIN_LEFT: i64 = 6;
const PIN_RIGHT: i64 = 7;

bitflags! {
    /// Bit flags for modifier keys. Bit 17 = shift, bit 18 = ctrl, bit 19 = option,
    /// bit 20 = cmd, bit 21 = numpad (which Warp does not store as a modifier).
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub struct Flags: u32 {
        const CTRL = 1 << 18;
        const ALT = 1 << 19;
        const SHIFT = 1 << 17;
        const CMD = 1 << 20;

        /// Bit mask for the non-modifier lower bits of iTerm's modifier flag.
        const LOWER_BIT_MASK = (!0u16) as u32 | 1 << 16;

        // Since we are using an external source, set all bits as "known."
        // See: https://docs.rs/bitflags/latest/bitflags/#externally-defined-flags
        const _ = !0;
    }
}

/// The iTerm theme data that we want to import.
///
/// We [`Box`] the values here to keep this enum smaller (and the two variants)
/// similarly-sized, as [`ITermTheme`] is 240 bytes (quite large).
#[derive(Debug, PartialEq)]
enum ITermThemeType {
    LightAndDark {
        light: Box<ITermTheme>,
        dark: Box<ITermTheme>,
    },
    Single(Box<ITermTheme>),
}

impl TryFrom<ITermThemeType> for ThemeType {
    type Error = ThemeError;
    fn try_from(theme_type: ITermThemeType) -> Result<Self, Self::Error> {
        let (default_light, default_dark) = default_iterm_themes();
        match theme_type {
            ITermThemeType::LightAndDark { light, dark } => Ok(ThemeType::LightAndDark {
                light: light.into_warp_theme(" (Light)", &default_light)?,
                dark: dark.into_warp_theme(" (Dark)", &default_dark)?,
            }),
            ITermThemeType::Single(normal) => Ok(ThemeType::Single(
                normal.into_warp_theme("", &default_dark)?,
            )),
        }
    }
}

#[derive(Debug, Default, PartialEq)]
struct ITermTheme {
    terminal_colors: Vec<Option<Dictionary>>,
    foreground: Option<Dictionary>,
    background: Option<Dictionary>,
    cursor: Option<Dictionary>,
}

impl ITermTheme {
    fn from_dictionary(dict: &mut Dictionary, suffix: &'static str) -> Self {
        ITermTheme {
            terminal_colors: (0..16)
                .map(|color_idx| {
                    dict.remove(format!("Ansi {color_idx} Color{suffix}").as_str())
                        .and_then(|value| value.into_dictionary())
                })
                .collect(),
            foreground: dict
                .remove(format!("Foreground Color{suffix}").as_str())
                .and_then(|value| value.into_dictionary()),
            background: dict
                .remove(format!("Background Color{suffix}").as_str())
                .and_then(|value| value.into_dictionary()),
            cursor: dict
                .remove(format!("Cursor Color{suffix}").as_str())
                .and_then(|value| value.into_dictionary()),
        }
    }

    fn into_warp_theme(
        mut self,
        suffix: &'static str,
        default_theme: &ITermTheme,
    ) -> Result<WarpTheme, ThemeError> {
        if self.foreground == default_theme.foreground
            || self.background == default_theme.background
        {
            return Err(ThemeError::MissingValueError);
        }

        let background = color_dictionary_to_coloru(self.background.as_mut())?;
        let foreground = color_dictionary_to_coloru(self.foreground.as_mut())?;
        let cursor = color_dictionary_to_coloru(self.cursor.as_mut())?;
        let bright = AnsiColors {
            black: color_dictionary_to_coloru(self.terminal_colors[8].as_mut())?.into(),
            red: color_dictionary_to_coloru(self.terminal_colors[9].as_mut())?.into(),
            green: color_dictionary_to_coloru(self.terminal_colors[10].as_mut())?.into(),
            yellow: color_dictionary_to_coloru(self.terminal_colors[11].as_mut())?.into(),
            blue: color_dictionary_to_coloru(self.terminal_colors[12].as_mut())?.into(),
            magenta: color_dictionary_to_coloru(self.terminal_colors[13].as_mut())?.into(),
            cyan: color_dictionary_to_coloru(self.terminal_colors[14].as_mut())?.into(),
            white: color_dictionary_to_coloru(self.terminal_colors[15].as_mut())?.into(),
        };

        let accent = calculate_accent_color(background, foreground, cursor, bright);

        Ok(WarpTheme::new(
            background.into(),
            foreground,
            accent.into(),
            Some(cursor.into()),
            None,
            TerminalColors {
                normal: AnsiColors {
                    black: color_dictionary_to_coloru(self.terminal_colors[0].as_mut())?.into(),
                    red: color_dictionary_to_coloru(self.terminal_colors[1].as_mut())?.into(),
                    green: color_dictionary_to_coloru(self.terminal_colors[2].as_mut())?.into(),
                    yellow: color_dictionary_to_coloru(self.terminal_colors[3].as_mut())?.into(),
                    blue: color_dictionary_to_coloru(self.terminal_colors[4].as_mut())?.into(),
                    magenta: color_dictionary_to_coloru(self.terminal_colors[5].as_mut())?.into(),
                    cyan: color_dictionary_to_coloru(self.terminal_colors[6].as_mut())?.into(),
                    white: color_dictionary_to_coloru(self.terminal_colors[7].as_mut())?.into(),
                },
                bright,
            },
            None,
            Some(format!("Imported iTerm Theme{suffix}")),
        ))
    }
}

fn color_dictionary_to_coloru(dict: Option<&mut Dictionary>) -> Result<ColorU, ThemeError> {
    let dict = dict.ok_or(ThemeError::MissingValueError)?;
    let srgb_color = Srgba::new(
        dict.remove("Red Component")
            .and_then(|value| value.as_real())
            .ok_or(ThemeError::MissingValueError)?,
        dict.remove("Green Component")
            .and_then(|value| value.as_real())
            .ok_or(ThemeError::MissingValueError)?,
        dict.remove("Blue Component")
            .and_then(|value| value.as_real())
            .ok_or(ThemeError::MissingValueError)?,
        dict.remove("Alpha Component")
            .and_then(|value| value.as_real())
            .unwrap_or(1.),
    )
    .into_format::<u8, u8>();

    Ok(ColorU {
        r: srgb_color.red,
        g: srgb_color.green,
        b: srgb_color.blue,
        a: srgb_color.alpha,
    })
}

/// How iTerm decides the working directory for new sessions.
#[derive(Debug, PartialEq)]
pub enum ITermWorkingDirectoryStrategy {
    /// Use the same working directory for all new sessions.
    Simple(ITermWorkingDirectory),
    /// Use a different working directory for different types of new sessions.
    Advanced {
        new_window: ITermWorkingDirectory,
        new_tab: ITermWorkingDirectory,
        new_pane: ITermWorkingDirectory,
    },
}

impl From<ITermWorkingDirectoryStrategy> for WorkingDirectoryConfig {
    fn from(value: ITermWorkingDirectoryStrategy) -> Self {
        match value {
            ITermWorkingDirectoryStrategy::Simple(strategy) => WorkingDirectoryConfig {
                advanced_mode: false,
                global: strategy.into(),
                ..Default::default()
            },
            ITermWorkingDirectoryStrategy::Advanced {
                new_window,
                new_tab,
                new_pane,
            } => WorkingDirectoryConfig {
                advanced_mode: true,
                new_window: new_window.into(),
                new_tab: new_tab.into(),
                split_pane: new_pane.into(),
                ..Default::default()
            },
        }
    }
}

/// Choices for which working directory new sessions use.
#[derive(Debug, PartialEq)]
pub enum ITermWorkingDirectory {
    Home,
    ReuseLast,
    Custom(String),
}

impl ITermWorkingDirectory {
    pub fn from_str(strategy: String, directory: String) -> ITermWorkingDirectory {
        match strategy.as_str() {
            "No" => ITermWorkingDirectory::Home,
            "Recycle" => ITermWorkingDirectory::ReuseLast,
            "Yes" => ITermWorkingDirectory::Custom(directory),
            _ => ITermWorkingDirectory::Home,
        }
    }
}

impl From<ITermWorkingDirectory> for WorkingDirectoryPerSourceConfig {
    fn from(value: ITermWorkingDirectory) -> Self {
        match value {
            ITermWorkingDirectory::Home => WorkingDirectoryPerSourceConfig {
                mode: WorkingDirectoryMode::HomeDir,
                custom_dir: "".to_string(),
            },
            ITermWorkingDirectory::ReuseLast => WorkingDirectoryPerSourceConfig {
                mode: WorkingDirectoryMode::PreviousDir,
                custom_dir: "".to_string(),
            },
            ITermWorkingDirectory::Custom(path) => WorkingDirectoryPerSourceConfig {
                mode: WorkingDirectoryMode::CustomDir,
                custom_dir: path,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ITermKeystroke {
    key: String,
    modifier: u64,
}

impl TryFrom<ITermKeystroke> for Keystroke {
    type Error = HotkeyError;
    fn try_from(value: ITermKeystroke) -> Result<Self, Self::Error> {
        // Because we have an "all" flag, this should always be Some(x).
        let modifier_flags = Flags::from_bits(value.modifier as u32).unwrap_or(Flags::empty());
        if modifier_flags & (Flags::LOWER_BIT_MASK) != Flags::empty() {
            log::info!("Unknown modifier for keystroke {value:?}");
            // Don't return an error, since iTerm uses lower bit flags for non-modifier related data.
        }
        let key = match unicode_char_to_key(value.key.chars().next().unwrap_or_default() as u16) {
            Some(key) => key.to_string(),
            None => value.key,
        };
        Ok(Keystroke {
            ctrl: modifier_flags.contains(Flags::CTRL),
            alt: modifier_flags.contains(Flags::ALT),
            shift: modifier_flags.contains(Flags::SHIFT),
            cmd: modifier_flags.contains(Flags::CMD),
            // Neither Warp nor iTerm supports Meta in global hotkeys.
            meta: false,
            key,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ITermGlobalHotkeyWindow {
    keystroke: ITermKeystroke,
    autohide: bool,
    /// Which screen the hotkey window should open on. -1 = any screen,
    /// -2 = screen with cursor (not supported in Warp), and >= 0 is the index of the screen.
    screen: i64,
    /// How the quake window displays. 2 is pin to top, 5 is bottom, 6 is left, and 7 is right.
    screen_type: i64,
}

impl ITermGlobalHotkeyWindow {
    pub fn new(hotkey_window: &Dictionary) -> Self {
        ITermGlobalHotkeyWindow {
            keystroke: ITermKeystroke {
                key: hotkey_window
                    .get("HotKey Characters Ignoring Modifiers")
                    .and_then(|characters| characters.as_string())
                    .unwrap_or_default()
                    .to_string(),
                modifier: hotkey_window
                    .get("HotKey Modifier Flags")
                    .and_then(|modifier| modifier.as_unsigned_integer())
                    .unwrap_or_default(),
            },
            autohide: hotkey_window
                .get("HotKey Window AutoHides")
                .and_then(|autohide| autohide.as_boolean())
                .unwrap_or(true),
            screen: hotkey_window
                .get("Screen")
                .and_then(|screen| screen.as_signed_integer())
                .unwrap_or(-1),
            screen_type: hotkey_window
                .get("Window Type")
                .and_then(|window_type| window_type.as_signed_integer())
                .unwrap_or(-1),
        }
    }
}

impl TryFrom<ITermGlobalHotkeyWindow> for GlobalHotkey {
    type Error = HotkeyError;
    fn try_from(value: ITermGlobalHotkeyWindow) -> Result<Self, Self::Error> {
        Ok(GlobalHotkey::QuakeMode(QuakeModeWindow {
            keystroke: TryInto::<Keystroke>::try_into(value.keystroke)?,
            autohide: value.autohide,
            screen: if value.screen == 0 {
                Some(DisplayIdx::Primary)
            } else if value.screen > 0 {
                if let Ok(screen_idx) = (value.screen - 1).try_into() {
                    Some(DisplayIdx::External(screen_idx))
                } else {
                    return Err(HotkeyError::UnsupportedWindowType);
                }
            } else if value.screen == -1 {
                None
            } else {
                return Err(HotkeyError::UnsupportedWindowType);
            },
            pin_position: match value.screen_type {
                PIN_TOP => QuakeModePinPosition::Top,
                PIN_BOTTOM => QuakeModePinPosition::Bottom,
                PIN_LEFT => QuakeModePinPosition::Left,
                PIN_RIGHT => QuakeModePinPosition::Right,
                screen_type => {
                    // We filter out dedicated hotkey windows that are not supported earlier,
                    // so this profile must be the default profile.
                    log::info!(
                        "Imported quake mode profile has unsupported window type {screen_type}"
                    );
                    QuakeModePinPosition::Top
                }
            },
        }))
    }
}

#[derive(Debug, PartialEq)]
pub struct ITermProfile {
    theme: ITermThemeType,
    profile_name: Option<String>,
    left_option_as_meta: bool,
    right_option_as_meta: bool,
    mouse_reporting: bool,
    scroll_reporting: bool,
    font_name: Option<String>,
    font_size: Option<String>,
    default_shell: Option<String>,
    working_directory: Option<ITermWorkingDirectoryStrategy>,
    hotkey_windows: Vec<ITermGlobalHotkeyWindow>,
    activation_keystroke: Option<ITermKeystroke>,
    columns: Option<u64>,
    rows: Option<u64>,
    transparency: Option<f64>,
    copy_on_select: Option<bool>,
    blur_radius: Option<f64>,
}

impl ITermProfile {
    pub fn from_dictionary(
        mut dict: Dictionary,
        mut hotkey_windows: Vec<ITermGlobalHotkeyWindow>,
        activation_keystroke: Option<ITermKeystroke>,
        copy_on_select: Option<bool>,
    ) -> Self {
        let theme_type = match dict
            .remove("Use Separate Colors for Light and Dark Mode")
            .and_then(|value| value.as_boolean())
            .unwrap_or(false)
        {
            true => ITermThemeType::LightAndDark {
                light: Box::new(ITermTheme::from_dictionary(&mut dict, " (Light)")),
                dark: Box::new(ITermTheme::from_dictionary(&mut dict, " (Dark)")),
            },
            false => ITermThemeType::Single(Box::new(ITermTheme::from_dictionary(&mut dict, ""))),
        };

        let mouse_reporting_enabled = dict
            .remove("Mouse Reporting")
            .and_then(|reporting| reporting.as_boolean())
            .unwrap_or(true);

        let (font_family, font_size) = dict
            .remove("Normal Font")
            .and_then(|font| font.into_string())
            .and_then(|font| {
                // iTerm stores fonts in the format "{font-name-and-style} {font-size}".
                // Get (font_name, font_size) as a tuple.
                font.split(' ')
                    .take(2)
                    .map(|element| element.to_owned())
                    .collect_tuple::<(String, String)>()
            })
            .unzip();

        let working_directory = match dict
            .remove("Custom Directory")
            .and_then(|value| value.into_string())
            .unwrap_or_default()
            .as_str()
        {
            strategy @ ("Yes" | "No" | "Recycle") => {
                ITermWorkingDirectoryStrategy::Simple(ITermWorkingDirectory::from_str(
                    strategy.to_string(),
                    dict.remove("Working Directory")
                        .and_then(|value| value.into_string())
                        .unwrap_or_default(),
                ))
            }
            "Advanced" => ITermWorkingDirectoryStrategy::Advanced {
                new_window: ITermWorkingDirectory::from_str(
                    dict.remove("AWDS Window Option")
                        .and_then(|value| value.into_string())
                        .unwrap_or_default(),
                    dict.remove("AWDS Window Directory")
                        .and_then(|value| value.into_string())
                        .unwrap_or_default(),
                ),
                new_tab: ITermWorkingDirectory::from_str(
                    dict.remove("AWDS Tab Option")
                        .and_then(|value| value.into_string())
                        .unwrap_or_default(),
                    dict.remove("AWDS Tab Directory")
                        .and_then(|value| value.into_string())
                        .unwrap_or_default(),
                ),
                new_pane: ITermWorkingDirectory::from_str(
                    dict.remove("AWDS Pane Option")
                        .and_then(|value| value.into_string())
                        .unwrap_or_default(),
                    dict.remove("AWDS Pane Directory")
                        .and_then(|value| value.into_string())
                        .unwrap_or_default(),
                ),
            },
            &_ => ITermWorkingDirectoryStrategy::Simple(ITermWorkingDirectory::Home),
        };

        if hotkey_windows.is_empty()
            && dict
                .get("Has Hotkey")
                .and_then(|has_hotkey_value| has_hotkey_value.as_boolean())
                .is_some_and(|has_hotkey| has_hotkey)
        {
            hotkey_windows.push(ITermGlobalHotkeyWindow::new(&dict));
        }

        ITermProfile {
            theme: theme_type,
            profile_name: dict.remove("Name").and_then(|name| name.into_string()),
            left_option_as_meta: dict
                .remove("Option Key Sends")
                .map(|value| value.as_unsigned_integer() == Some(1))
                .unwrap_or(false),
            right_option_as_meta: dict
                .remove("Right Option Key Sends")
                .map(|value| value.as_unsigned_integer() == Some(1))
                .unwrap_or(false),
            mouse_reporting: mouse_reporting_enabled
                && dict
                    .remove("Mouse Reporting allow clicks and drags")
                    .and_then(|reporting| reporting.as_boolean())
                    .unwrap_or(true),
            // Allow scroll reporting to be set without mouse reporting
            scroll_reporting: dict
                .remove("Mouse Reporting allow mouse wheel")
                .and_then(|reporting| reporting.as_boolean())
                .unwrap_or(true),
            font_name: font_family,
            font_size,
            default_shell: dict.remove("Custom Command").and_then(|custom_command| {
                if custom_command.into_string().is_some_and(|command_pref| {
                    command_pref == "Custom Shell" || command_pref == "Yes"
                }) {
                    dict.remove("Command")
                        .and_then(|command| command.into_string())
                } else {
                    None
                }
            }),
            working_directory: Some(working_directory),
            hotkey_windows,
            activation_keystroke,
            copy_on_select,
            rows: dict
                .remove("Rows")
                .and_then(|rows| rows.as_unsigned_integer()),
            columns: dict
                .remove("Columns")
                .and_then(|cols| cols.as_unsigned_integer()),
            transparency: dict
                .remove("Transparency")
                .and_then(|transparency| transparency.as_real()),
            blur_radius: if dict
                .remove("Blur")
                .and_then(|blur| blur.as_boolean())
                .unwrap_or_default()
            {
                dict.remove("Blur Radius")
                    .and_then(|blur_radius| blur_radius.as_real())
            } else {
                None
            },
        }
    }
}

#[async_trait]
impl ParseableConfig for ITermProfile {
    async fn from_file(path: PathBuf) -> Result<Vec<Self>, ConfigError> {
        let Some(mut dict) = Value::from_file(path.clone())
            .map_err(|_| ConfigError::MalformattedFileError(path.clone()))?
            .into_dictionary()
        else {
            return Err(ConfigError::MalformattedFileError(path));
        };

        if dict
            .remove("LoadPrefsFromCustomFolder")
            .and_then(|value| value.as_boolean())
            .unwrap_or(false)
        {
            let Some(custom_path) = dict
                .remove("PrefsCustomFolder")
                .and_then(|folder| folder.into_string())
            else {
                return Err(ConfigError::MalformattedFileError(path));
            };
            let Some(custom_dict) = Value::from_file(
                PathBuf::from(custom_path.clone()).join("com.googlecode.iterm2.plist"),
            )
            .map_err(|_| {
                ConfigError::MalformattedFileError(
                    PathBuf::from(custom_path.clone()).join("com.googlecode.iterm2.plist"),
                )
            })?
            .into_dictionary() else {
                return Err(ConfigError::MalformattedFileError(PathBuf::from(
                    custom_path,
                )));
            };
            dict = custom_dict;
        }

        let Some(default_guid) = dict
            .remove("Default Bookmark Guid")
            .and_then(|guid| guid.into_string())
        else {
            return Err(ConfigError::MalformattedFileError(path));
        };

        let Some(profiles) = dict
            .remove("New Bookmarks")
            .and_then(|profiles| profiles.into_array())
            .and_then(|profiles| {
                profiles
                    .into_iter()
                    .map(|profile| profile.into_dictionary())
                    .collect::<Option<Vec<_>>>()
            })
        else {
            return Err(ConfigError::MalformattedFileError(path));
        };

        // Get candidate hotkey windows.
        let hotkey_windows = profiles
            .iter()
            .filter_map(|hotkey_window| {
                if !(hotkey_window
                    .get("Has Hotkey")
                    .and_then(|has_hotkey_value| has_hotkey_value.as_boolean())
                    .is_some_and(|has_hotkey| has_hotkey)
                    && hotkey_window
                        .get("Window Type")
                        .and_then(|window_type| window_type.as_signed_integer())
                        .is_some_and(|window_type| {
                            [PIN_TOP, PIN_BOTTOM, PIN_LEFT, PIN_RIGHT].contains(&window_type)
                        }))
                {
                    None
                } else {
                    Some(ITermGlobalHotkeyWindow::new(hotkey_window))
                }
            })
            .collect::<Vec<_>>();

        // Get the keystroke to show/hide all iTerm windows.
        let activation_keystroke = if dict
            .get("Hotkey")
            .and_then(|has_hotkey_value| has_hotkey_value.as_boolean())
            .is_some_and(|has_hotkey| has_hotkey)
        {
            dict.remove("HotkeyChar")
                .and_then(|value| value.as_unsigned_integer())
                .zip(
                    dict.remove("HotkeyModifiers")
                        .and_then(|value| value.as_unsigned_integer()),
                )
                .map(|(char, modifier)| ITermKeystroke {
                    key: char::from_u32(char as u32).unwrap_or_default().to_string(),
                    modifier,
                })
        } else {
            None
        };

        let copy_on_select = dict
            .remove("CopySelection")
            .and_then(|value| value.as_boolean());

        Ok(profiles
            .into_iter()
            .filter(|profile| {
                profile
                    .get("Guid")
                    .and_then(|guid| guid.as_string())
                    .is_some_and(|guid| guid == default_guid)
            })
            .map(|dictionary| {
                ITermProfile::from_dictionary(
                    dictionary,
                    hotkey_windows.clone(),
                    activation_keystroke.clone(),
                    copy_on_select,
                )
            })
            .collect::<Vec<_>>())
    }

    fn parse(mut self, fonts: &[FontInfo]) -> Config {
        // iTerm stores its fonts with internal names and supports styles as default terminal text, whereas Warp changes fonts based on display name.
        // Only import a font if there is only one font whose iTerm name starts with the display name of a font Warp supports.
        let translated_font_name = fonts
            .iter()
            .find(|font_info| {
                self.font_name
                    .as_ref()
                    .is_some_and(|font_name| font_info.font_names.contains(font_name))
            })
            .map(|font_info: &FontInfo| font_info.family_name.to_owned());

        let hotkey_window = if self.hotkey_windows.len() == 1 {
            TryInto::<GlobalHotkey>::try_into(self.hotkey_windows.remove(0))
        } else if self.hotkey_windows.is_empty() {
            Err(HotkeyError::MissingHotkey)
        } else {
            Err(HotkeyError::MultipleHotkeys)
        };

        let hotkey_mode = match (
            hotkey_window,
            self.activation_keystroke
                .map(TryInto::<Keystroke>::try_into),
        ) {
            (Ok(_), Some(Ok(activation_keystroke))) => {
                log::info!("Found an activation keystroke and at least one quake mode window!");
                Ok(GlobalHotkey::Activation(activation_keystroke))
            }
            (Ok(hotkey_window), _) => Ok(hotkey_window),
            // We had multiple quake mode windows, so propagate the error up.
            (Err(HotkeyError::MultipleHotkeys), _) => Err(HotkeyError::MultipleHotkeys),
            (Err(_), Some(Ok(activation_keystroke))) => {
                Ok(GlobalHotkey::Activation(activation_keystroke))
            }
            (Err(_), _) => Err(HotkeyError::MissingHotkey),
        };

        let mouse_and_scroll_reporting = match (self.mouse_reporting, self.scroll_reporting) {
            // Since this is the Warp default, return None.
            (true, true) => None,
            (mouse_reporting, scroll_reporting) => Some(MouseAndScrollReporting {
                mouse_reporting,
                scroll_reporting,
            }),
        };

        let opacity_settings = OpacitySettings {
            opacity: self
                .transparency
                .map(|transparency| ((1. - transparency) * 100.) as u8),
            blur_radius: self.blur_radius.map(|blur_radius| blur_radius as u8),
        };

        let default_shell = self
            .default_shell
            .filter(|shell| is_valid_path_or_command_for_supported_shell(shell))
            .map(StartupShell::Custom);

        let font = ImportedFont {
            family: translated_font_name,
            size: self
                .font_size
                .and_then(|font_size| font_size.parse::<f32>().ok()),
        };

        let window_size = (
            self.columns.map(|cols| cols as u16),
            self.rows.map(|rows| rows as u16),
        );

        let working_directory = self.working_directory.map(|workspace| workspace.into());

        let option_as_meta = ExtraMetaKeys {
            left_alt: self.left_option_as_meta,
            right_alt: self.right_option_as_meta,
        };

        Config {
            theme: ImportableSetting::new(self.theme.try_into(), SettingType::Theme),
            terminal_name: "iTerm2".to_string(),
            mouse_and_scroll_reporting: ImportableSetting::new(
                mouse_and_scroll_reporting,
                SettingType::MouseAndScrollReporting,
            ),
            option_as_meta: ImportableSetting::new(option_as_meta, SettingType::OptionAsMeta),
            description: self.profile_name.map(|name| format!("Profile: {name}")),
            font: ImportableSetting::new(font, SettingType::Font),
            default_shell: ImportableSetting::new(default_shell, SettingType::DefaultShell),
            working_directory: ImportableSetting::new(
                working_directory,
                SettingType::WorkingDirectory,
            ),
            hotkey_mode: ImportableSetting::new(hotkey_mode, SettingType::HotkeyMode),
            copy_on_select: ImportableSetting::new(self.copy_on_select, SettingType::CopyOnSelect),
            window_size: ImportableSetting::new(window_size, SettingType::WindowSize),
            cursor_blinking: ImportableSetting::new(None, SettingType::CursorBlinking),
            opacity: ImportableSetting::new(opacity_settings, SettingType::Opacity),
        }
    }

    fn default_paths() -> Vec<PathBuf> {
        vec![PathBuf::from(
            shellexpand::tilde("~/Library/Preferences/com.googlecode.iterm2.plist")
                .into_owned()
                .to_string(),
        )]
    }

    fn remove_default_values(mut self) -> Self {
        let (default_light, default_dark) = default_iterm_themes();

        self.theme = match self.theme {
            ITermThemeType::LightAndDark { light, dark } => {
                match (light == default_light, dark == default_dark) {
                    (true, true) => ITermThemeType::Single(Box::default()),
                    (true, false) => ITermThemeType::Single(dark),
                    (false, true) => ITermThemeType::Single(light),
                    (false, false) => {
                        if light == dark {
                            ITermThemeType::Single(dark)
                        } else {
                            ITermThemeType::LightAndDark { light, dark }
                        }
                    }
                }
            }
            ITermThemeType::Single(theme) => {
                if theme == default_dark {
                    ITermThemeType::Single(Box::default())
                } else {
                    ITermThemeType::Single(theme)
                }
            }
        };

        let default_profile = ITermProfile::default();

        // Don't import font family if it is the default monospace font family.
        if self.font_name == default_profile.font_name
            || self.font_name == Some(DEFAULT_MONOSPACE_FONT_NAME.to_string())
        {
            self.font_name = None;
        }

        // Don't import font size if it is the default monospace font size.
        if self.font_size == default_profile.font_size
            || self.font_size == Some((DEFAULT_MONOSPACE_FONT_SIZE as u32).to_string())
        {
            self.font_size = None;
        }

        if self.working_directory == default_profile.working_directory
            || self.working_directory == Some(WARP_DEFAULT_WORKING_DIRECTORY)
        {
            self.working_directory = None;
        }

        // Warp's default is not to open windows with a custom size,
        // so there is nothing to check against.
        if self.rows == default_profile.rows {
            self.rows = None;
        }
        if self.columns == default_profile.columns {
            self.columns = None;
        }
        // iTerm's presets are the same as Warp's
        if self.transparency == default_profile.transparency {
            self.transparency = None;
        }

        if self.copy_on_select == default_profile.copy_on_select {
            self.copy_on_select = None;
        }

        self
    }
}

#[cfg(test)]
#[path = "iterm_parser_tests.rs"]
mod tests;

impl Default for ITermProfile {
    fn default() -> Self {
        Self {
            right_option_as_meta: false,
            left_option_as_meta: false,
            mouse_reporting: true,
            scroll_reporting: true,
            font_name: Some(ITERM_DEFAULT_MONOSPACE_FONT_FAMILY.to_string()),
            font_size: Some(ITERM_DEFAULT_MONOSPACE_FONT_SIZE.to_string()),
            theme: ITermThemeType::LightAndDark {
                light: default_light_theme(),
                dark: default_dark_theme(),
            },
            profile_name: Some("Default".to_string()),
            default_shell: None,
            working_directory: Some(ITermWorkingDirectoryStrategy::Simple(
                ITermWorkingDirectory::Home,
            )),
            hotkey_windows: vec![],
            activation_keystroke: None,
            rows: Some(25),
            columns: Some(80),
            copy_on_select: Some(true),
            transparency: Some(0.),
            blur_radius: None,
        }
    }
}

fn default_iterm_themes() -> (Box<ITermTheme>, Box<ITermTheme>) {
    (default_light_theme(), default_dark_theme())
}

fn default_light_theme() -> Box<ITermTheme> {
    Box::new(ITermTheme {
        terminal_colors: vec![
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.0784313753247261)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.11764705926179886)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.09803921729326248)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.7074432373046875)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.16300037503242493)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.23660069704055786)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.0)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.0)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.7607843279838562)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.7805864810943604)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.0)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.7695948481559753)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.15404300391674042)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.7821617722511292)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.2647435665130615)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.752197265625)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.7449436187744141)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.24931684136390686)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.0)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.7816620469093323)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.7742590308189392)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.7810397744178772)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.7810482978820801)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.7810582518577576)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.4078176021575928)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.4078223705291748)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.40782788395881653)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.8659515380859375)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.45833224058151245)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.4752407670021057)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.3450070321559906)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.5654193758964539)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.9042816162109375)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.9259033203125)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.0)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.8833775520324707)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.6534907817840576)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.9485321044921875)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.6704471707344055)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.8821563720703125)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.8821563720703125)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.4927266538143158)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.3759753108024597)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(1.0)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.9926329255104065)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.9999960064888)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(1.0)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(1.0)),
            ])),
        ],
        foreground: Some(Dictionary::from_iter([
            ("Red Component", Value::Real(0.06274509803921569)),
            ("Color Space", Value::String("sRGB".to_string())),
            ("Blue Component", Value::Real(0.06274509803921569)),
            ("Alpha Component", Value::Real(1.0)),
            ("Green Component", Value::Real(0.06274509803921569)),
        ])),
        background: Some(Dictionary::from_iter([
            ("Red Component", Value::Real(0.98)),
            ("Color Space", Value::String("sRGB".to_string())),
            ("Blue Component", Value::Real(0.98)),
            ("Alpha Component", Value::Real(1.0)),
            ("Green Component", Value::Real(0.98)),
        ])),
        cursor: Some(Dictionary::from_iter([
            ("Red Component", Value::Real(0.0)),
            ("Color Space", Value::String("sRGB".to_string())),
            ("Blue Component", Value::Real(0.0)),
            ("Alpha Component", Value::Real(1.0)),
            ("Green Component", Value::Real(0.0)),
        ])),
    })
}

fn default_dark_theme() -> Box<ITermTheme> {
    Box::new(ITermTheme {
        terminal_colors: vec![
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.0784313753247261)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.11764705926179886)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.09803921729326248)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.7074432373046875)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.16300037503242493)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.23660069704055786)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.0)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.0)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.7607843279838562)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.7805864810943604)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.0)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.7695948481559753)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.15404300391674042)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.7821617722511292)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.2647435665130615)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.752197265625)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.7449436187744141)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.24931684136390686)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.0)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.7816620469093323)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.7742590308189392)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.7810397744178772)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.7810482978820801)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.7810582518577576)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.4078176021575928)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.4078223705291748)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.40782788395881653)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.8659515380859375)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.45833224058151245)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.4752407670021057)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.3450070321559906)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.5654193758964539)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.9042816162109375)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.9259033203125)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.0)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.8833775520324707)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.6534907817840576)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.9485321044921875)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.6704471707344055)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.8821563720703125)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(0.8821563720703125)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.4927266538143158)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.3759753108024597)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(1.0)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(0.9926329255104065)),
            ])),
            Some(Dictionary::from_iter([
                ("Red Component", Value::Real(0.9999960064888)),
                ("Color Space", Value::String("sRGB".to_string())),
                ("Blue Component", Value::Real(1.0)),
                ("Alpha Component", Value::Real(1.0)),
                ("Green Component", Value::Real(1.0)),
            ])),
        ],
        foreground: Some(Dictionary::from_iter([
            ("Red Component", Value::Real(0.8619791269302368)),
            ("Color Space", Value::String("sRGB".to_string())),
            ("Blue Component", Value::Real(0.8619885444641113)),
            ("Alpha Component", Value::Real(1.0)),
            ("Green Component", Value::Real(0.86199951171875)),
        ])),
        background: Some(Dictionary::from_iter([
            ("Red Component", Value::Real(0.0806884765625)),
            ("Color Space", Value::String("sRGB".to_string())),
            ("Blue Component", Value::Real(0.12103271484375)),
            ("Alpha Component", Value::Real(1.0)),
            ("Green Component", Value::Real(0.09911105036735535)),
        ])),
        cursor: Some(Dictionary::from_iter([
            ("Red Component", Value::Real(0.9999763369560242)),
            ("Color Space", Value::String("sRGB".to_string())),
            ("Blue Component", Value::Real(0.9999872446060181)),
            ("Alpha Component", Value::Real(1.0)),
            ("Green Component", Value::Real(1.0)),
        ])),
    })
}
