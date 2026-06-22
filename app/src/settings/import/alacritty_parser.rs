use std::env;
use std::io::ErrorKind;
use std::path::PathBuf;

use async_recursion::async_recursion;
use async_trait::async_trait;
use pathfinder_color::ColorU;
use serde::Deserialize;
use warp_core::ui::color::hex_color::coloru_from_hex_string;
use warp_core::ui::theme::{AnsiColor, AnsiColors, TerminalColors, WarpTheme};
use warpui::fonts::FontInfo;

use super::config::{
    calculate_accent_color, Config, ConfigError, ImportableSetting, ParseableConfig, SettingType,
    ThemeType,
};
use crate::settings::import::config::ThemeError;

type AlacrittyColor = String;
const CONFIG_DEPTH_LIMIT: u8 = 5;

// Constants for Alacritty's defaults: see the "Colors" section
// of https://alacritty.org/config-alacritty.html
pub const DEFAULT_ALACRITTY_FOREGROUND: ColorU = ColorU {
    r: 0xd8,
    g: 0xd8,
    b: 0xd8,
    a: 255,
};

pub const DEFAULT_ALACRITTY_BACKGROUND: ColorU = ColorU {
    r: 0x18,
    g: 0x18,
    b: 0x18,
    a: 255,
};

pub const DEFAULT_ALACRITTY_BRIGHT_COLORS: AnsiColors = AnsiColors {
    black: AnsiColor {
        r: 0x6b,
        g: 0x6b,
        b: 0x6b,
    },
    red: AnsiColor {
        r: 0xc5,
        g: 0x55,
        b: 0x55,
    },
    green: AnsiColor {
        r: 0xaa,
        g: 0xc4,
        b: 0x74,
    },
    yellow: AnsiColor {
        r: 0xfe,
        g: 0xca,
        b: 0x88,
    },
    blue: AnsiColor {
        r: 0x82,
        g: 0xb8,
        b: 0xc8,
    },
    magenta: AnsiColor {
        r: 0xc2,
        g: 0x8c,
        b: 0xb8,
    },
    cyan: AnsiColor {
        r: 0x93,
        g: 0xd3,
        b: 0xc3,
    },
    white: AnsiColor {
        r: 0xf8,
        g: 0xf8,
        b: 0xf8,
    },
};

pub const DEFAULT_ALACRITTY_NORMAL_COLORS: AnsiColors = AnsiColors {
    black: AnsiColor {
        r: 0x18,
        g: 0x18,
        b: 0x18,
    },
    red: AnsiColor {
        r: 0xac,
        g: 0x42,
        b: 0x42,
    },
    green: AnsiColor {
        r: 0x90,
        g: 0xa9,
        b: 0x59,
    },
    yellow: AnsiColor {
        r: 0xf4,
        g: 0xbf,
        b: 0x75,
    },
    blue: AnsiColor {
        r: 0x6a,
        g: 0x9f,
        b: 0xb5,
    },
    magenta: AnsiColor {
        r: 0xaa,
        g: 0x75,
        b: 0x9f,
    },
    cyan: AnsiColor {
        r: 0x75,
        g: 0xb5,
        b: 0xaa,
    },
    white: AnsiColor {
        r: 0xd8,
        g: 0xd8,
        b: 0xd8,
    },
};

/// `RecursivelyParseable` is a trait that bundles recursive functions over the different
/// data structures in our representation of Alacritty's config.
trait RecursivelyParseable {
    /// Fills in any None fields from `self` with values from `other`.
    fn merge_left(self, other: Self) -> Self;
}

#[derive(Clone, Default, Deserialize, PartialEq)]
pub struct AlacrittyTheme {
    primary: Option<PrimaryAlacrittyColors>,
    normal: Option<AlacrittyColors>,
    bright: Option<AlacrittyColors>,
    cursor: Option<AlacrittyCursorColor>,
}

/// Since Alacritty's config stores the cursor color in colors.cursor.cursor, we use this struct
/// to match Alacritty's keys exactly.
#[derive(Clone, Default, Deserialize, PartialEq)]
pub struct AlacrittyCursorColor {
    cursor: Option<AlacrittyColor>,
}

#[derive(Clone, Default, Deserialize, PartialEq)]
pub struct PrimaryAlacrittyColors {
    foreground: Option<AlacrittyColor>,
    background: Option<AlacrittyColor>,
}

#[derive(Clone, Default, Deserialize, PartialEq)]
pub struct AlacrittyColors {
    black: Option<AlacrittyColor>,
    red: Option<AlacrittyColor>,
    green: Option<AlacrittyColor>,
    yellow: Option<AlacrittyColor>,
    blue: Option<AlacrittyColor>,
    magenta: Option<AlacrittyColor>,
    cyan: Option<AlacrittyColor>,
    white: Option<AlacrittyColor>,
}

#[derive(Clone, Default, Deserialize, PartialEq)]
pub struct AlacrittyConfig {
    colors: Option<AlacrittyTheme>,
    import: Option<Vec<String>>,
}

impl RecursivelyParseable for AlacrittyConfig {
    fn merge_left(mut self, other: Self) -> Self {
        self.colors = self
            .colors
            .map(|inner| inner.merge_left(other.colors.clone().unwrap_or_default()))
            .or(other.colors);

        // Concatenate import lists if we can. Otherwise, take the other import list.
        if let Some(import) = self.import {
            self.import = Some(
                import
                    .into_iter()
                    .chain(other.import.unwrap_or_default())
                    .collect(),
            );
        } else {
            self.import = other.import;
        }
        self
    }
}

impl RecursivelyParseable for AlacrittyColors {
    fn merge_left(mut self, other: Self) -> Self {
        self.black = self.black.or(other.black);
        self.red = self.red.or(other.red);
        self.green = self.green.or(other.green);
        self.yellow = self.yellow.or(other.yellow);
        self.blue = self.blue.or(other.blue);
        self.magenta = self.magenta.or(other.magenta);
        self.cyan = self.cyan.or(other.cyan);
        self.white = self.white.or(other.white);
        self
    }
}

impl RecursivelyParseable for PrimaryAlacrittyColors {
    fn merge_left(mut self, other: Self) -> Self {
        self.foreground = self.foreground.or(other.foreground);
        self.background = self.background.or(other.background);
        self
    }
}

impl RecursivelyParseable for AlacrittyTheme {
    fn merge_left(mut self, other: Self) -> Self {
        self.primary = self
            .primary
            .map(|inner| inner.merge_left(other.primary.clone().unwrap_or_default()))
            .or(other.primary);

        self.normal = self
            .normal
            .map(|inner| inner.merge_left(other.normal.clone().unwrap_or_default()))
            .or(other.normal);

        self.bright = self
            .bright
            .map(|inner| inner.merge_left(other.bright.clone().unwrap_or_default()))
            .or(other.bright);

        self.cursor = self
            .cursor
            .map(|inner| inner.merge_left(other.cursor.clone().unwrap_or_default()))
            .or(other.cursor);
        self
    }
}

impl RecursivelyParseable for AlacrittyCursorColor {
    fn merge_left(mut self, other: Self) -> Self {
        self.cursor = self.cursor.or(other.cursor);
        self
    }
}

/// Parses a hex string into an AnsiColor, returning an error only if
/// the hex string is present but malformatted.
fn parse_alacritty_color(
    color_string: Option<AlacrittyColor>,
) -> Result<Option<AnsiColor>, ThemeError> {
    let Some(color) = color_string else {
        return Ok(None);
    };
    if color.is_empty() {
        return Ok(None);
    }
    match coloru_from_hex_string(color.as_str()) {
        Ok(color) => Ok(Some(color.into())),
        Err(e) => Err(ThemeError::HexColorError(e)),
    }
}

impl AlacrittyTheme {
    fn parse(self) -> Result<ThemeType, ThemeError> {
        let primary = self.primary.unwrap_or_default();
        let foreground = parse_alacritty_color(primary.foreground)?
            .unwrap_or(DEFAULT_ALACRITTY_FOREGROUND.into());
        let background = parse_alacritty_color(primary.background)?
            .unwrap_or(DEFAULT_ALACRITTY_BACKGROUND.into());
        let cursor_color =
            parse_alacritty_color(self.cursor.unwrap_or_default().cursor)?.unwrap_or(foreground);

        // Create TerminalColors from the config, filling in any missing values from Alacritty's default
        // normal and bright colors.
        let terminal_colors = TerminalColors {
            normal: self
                .normal
                .unwrap_or_default()
                .into_ansi_with_default(DEFAULT_ALACRITTY_NORMAL_COLORS)?,
            bright: self
                .bright
                .unwrap_or_default()
                .into_ansi_with_default(DEFAULT_ALACRITTY_BRIGHT_COLORS)?,
        };

        if foreground == DEFAULT_ALACRITTY_FOREGROUND.into()
            || background == DEFAULT_ALACRITTY_BACKGROUND.into()
        {
            Err(ThemeError::MissingValueError)
        } else {
            let bright = terminal_colors.bright;
            let accent = calculate_accent_color(background, foreground, cursor_color, bright);
            Ok(ThemeType::Single(WarpTheme::new(
                background.into(),
                foreground.into(),
                accent.into(),
                Some(cursor_color.into()),
                None,
                terminal_colors,
                None,
                Some(String::from("Imported Alacritty Theme")),
            )))
        }
    }
}

impl AlacrittyColors {
    /// Returns terminal colors with Warp's default colors substituted in for any
    /// missing terminal colors.
    fn into_ansi_with_default(self, default: AnsiColors) -> Result<AnsiColors, ThemeError> {
        Ok(AnsiColors {
            black: parse_alacritty_color(self.black)?.unwrap_or(default.black),
            red: parse_alacritty_color(self.red)?.unwrap_or(default.red),
            green: parse_alacritty_color(self.green)?.unwrap_or(default.green),
            yellow: parse_alacritty_color(self.yellow)?.unwrap_or(default.yellow),
            blue: parse_alacritty_color(self.blue)?.unwrap_or(default.blue),
            magenta: parse_alacritty_color(self.magenta)?.unwrap_or(default.magenta),
            cyan: parse_alacritty_color(self.cyan)?.unwrap_or(default.cyan),
            white: parse_alacritty_color(self.white)?.unwrap_or(default.white),
        })
    }
}

#[async_trait]
impl ParseableConfig for AlacrittyConfig {
    fn parse(self, _font_info: &[FontInfo]) -> Config {
        Config {
            theme: ImportableSetting::new(self.parse_theme(), SettingType::Theme),
            terminal_name: "Alacritty".to_string(),
            ..Default::default()
        }
    }

    async fn from_file(path: PathBuf) -> Result<Vec<Self>, ConfigError> {
        Self::from_file_bounded_depth(path, 0).await
    }

    fn default_paths() -> Vec<std::path::PathBuf> {
        // We follow Alacritty's strategy described here: https://github.com/alacritty/alacritty?tab=readme-ov-file#configuration.
        // Since alacritty uses the `xdg` crate to read config files, we include paths in XDG_CONFIG_DIRS.

        // If we are on Windows, search the only path: %APPDATA%\alacritty\alacritty.toml.
        if cfg!(windows) {
            return dirs::config_dir()
                .map(|path| vec![path.join("alacritty").join("alacritty.toml")])
                .unwrap_or_default();
        }

        let mut file_paths = vec![];
        let mut second_file_paths = vec![];

        let xdg_config_dirs = env::var("XDG_CONFIG_DIRS")
            .ok()
            .filter(|val| !val.is_empty())
            .or_else(|| Some("/usr/local/share/:/usr/share/".to_string()));

        // Add to file_paths:
        // - $XDG_CONFIG_HOME/alacritty/alacritty.toml
        // Add to second_file_paths:
        // - $XDG_CONFIG_HOME/alacritty.toml
        if let Some(xdg_config_home) = dirs::config_dir() {
            file_paths.push(xdg_config_home.join("alacritty").join("alacritty.toml"));
            second_file_paths.push(xdg_config_home.join("alacritty.toml"));
        }

        // Add to file_paths:
        // - $XDG_CONFIG_DIRS/alacritty/alacritty.toml
        // Add to second_file_paths:
        // - $XDG_CONFIG_DIRS/alacritty.toml
        if let Some(xdg_config_dirs) = xdg_config_dirs.clone() {
            for dir in xdg_config_dirs.split(':') {
                if !dir.is_empty() {
                    file_paths.push(PathBuf::from(dir).join("alacritty").join("alacritty.toml"));
                    second_file_paths.push(PathBuf::from(dir).join("alacritty.toml"));
                }
            }
        }

        // Add second_file_paths to the end of file_paths to maintain the correct order.
        file_paths.extend(second_file_paths);

        // As a backup, check
        // - $HOME/.config/alacritty/alacritty.toml
        // - $HOME/.alacritty.toml
        if let Some(home) = dirs::home_dir() {
            file_paths.push(
                home.join(".config")
                    .join("alacritty")
                    .join("alacritty.toml"),
            );
            file_paths.push(home.join(".alacritty.toml"));
        }

        file_paths
    }

    fn remove_default_values(self) -> Self {
        // The default Alacritty config is an empty file,
        // so we don't need to remove anything.
        self
    }
}

impl AlacrittyConfig {
    /// Reads Alacritty configs asynchronously and folds in any imported configs up to a given depth.
    #[async_recursion]
    async fn from_file_bounded_depth(path: PathBuf, depth: u8) -> Result<Vec<Self>, ConfigError> {
        // Since Alacritty only reads configs up to depth 5,
        // return an empty config if we are at depth 5.
        if depth >= CONFIG_DEPTH_LIMIT {
            log::warn!(
                "Maximum configuration depth reached while parsing Alacritty configuration at {path:?}"
            );
            return Ok(vec![AlacrittyConfig::default()]);
        }
        let contents = match async_fs::read_to_string(path.clone()).await {
            Ok(string) => string,
            Err(e) => {
                if e.kind() == ErrorKind::NotFound {
                    return Err(ConfigError::FileNotFoundError);
                } else {
                    return Err(ConfigError::FileIOError(e));
                }
            }
        };

        let Ok(mut out) = toml::from_str::<AlacrittyConfig>(contents.as_str()) else {
            return Err(ConfigError::MalformattedFileError(path));
        };
        // Alacritty prioritizes settings set in the current config.
        // If a setting is not set in the highest-level config, it then looks to the imported configs.
        // It reads each imported config in order, effectively prioritizing the last config listed.
        // It does support tilde as the home directory, but not environment variables or relative paths.

        // Start with a config with None in all fields.
        let mut imported_config: AlacrittyConfig = Default::default();
        if let Some(ref imports) = out.import {
            // Reverse the iterator, so that the last configs listed get priority.
            for file_path in imports.iter().rev() {
                // Merge left, which replaces None values in the first argument
                // with values from the second argument.
                imported_config = imported_config.merge_left(
                    Self::from_file_bounded_depth(
                        PathBuf::from(shellexpand::tilde(file_path).into_owned().to_string()),
                        depth + 1,
                    )
                    .await?
                    .pop()
                    .unwrap_or_default(),
                );
            }
        }
        out = out.merge_left(imported_config);
        Ok(vec![out])
    }

    fn parse_theme(self) -> Result<ThemeType, ThemeError> {
        match self.colors {
            Some(colors) => Ok(colors.parse()?),
            None => Err(ThemeError::MissingValueError),
        }
    }
}

#[cfg(test)]
#[path = "alacritty_parser_tests.rs"]
mod tests;
