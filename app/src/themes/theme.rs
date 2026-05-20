use super::default_themes::*;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::iter::FromIterator;
use std::path::PathBuf;
use warp_core::ui::color::pick_foreground_color;
use warpui::assets::asset_cache::AssetSource;
use warpui::{
    color::ColorU,
    elements::{
        Align, Border, ConstrainedBox, Container, Element, Empty, Flex, ParentElement, Rect,
        Shrinkable, Stack, Text,
    },
    fonts::FamilyId,
};

use super::theme_creator::{pick_accent_color_from_options, top_colors_for_image};

pub use warp_core::ui::color::blend::Blend;
pub use warp_core::ui::theme::*;

const THUMBNAIL_MARGIN: f32 = 10.;

// We use the discriminant of enum variants to determine the order of theme types in the
// theme chooser view.
#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    Serialize,
    Deserialize,
    Hash,
    Eq,
    Ord,
    PartialOrd,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "The color theme.", rename_all = "snake_case")]
pub enum ThemeKind {
    // Need an alias for backwards-compatibility: Originally we only had a single reward theme
    // so it was named `ReferralReward`.
    #[serde(alias = "ReferralReward")]
    #[schemars(skip)]
    SentReferralReward,
    #[schemars(skip)]
    ReceivedReferralReward,
    #[schemars(description = "Adeberry")]
    Adeberry,
    #[schemars(description = "Phenomenon")]
    Phenomenon,
    #[default]
    #[schemars(description = "Dark")]
    Dark,
    #[schemars(description = "Dracula")]
    Dracula,
    #[schemars(description = "Fancy Dracula")]
    FancyDracula,
    #[schemars(description = "Cyber Wave")]
    CyberWave,
    #[schemars(description = "Solar Flare")]
    SolarFlare,
    #[schemars(description = "Solarized Dark")]
    SolarizedDark,
    #[schemars(description = "Willow Dream")]
    WillowDream,
    #[schemars(description = "Light")]
    Light,
    #[schemars(description = "Dark City")]
    DarkCity,
    #[schemars(description = "Gruvbox Dark")]
    GruvboxDark,
    #[schemars(description = "Red Rock")]
    RedRock,
    #[schemars(description = "Jellyfish")]
    JellyFish,
    #[schemars(description = "Leafy")]
    Leafy,
    #[schemars(description = "Koi")]
    Koi,
    #[schemars(description = "Solarized Light")]
    SolarizedLight,
    #[schemars(description = "Snowy")]
    Snowy,
    #[schemars(description = "Gruvbox Light")]
    GruvboxLight,
    #[schemars(description = "Pink City")]
    PinkCity,
    #[schemars(description = "Marble")]
    Marble,
    #[schemars(description = "A user-provided custom theme loaded from a file.")]
    Custom(CustomTheme),
    /// Base16 themes are a special case of custom themes with their own semantics for ANSI colors that override "bright" color variants.
    #[schemars(description = "A custom theme using the Base16 color scheme format.")]
    CustomBase16(CustomTheme),
    #[schemars(skip)]
    InMemory(InMemoryThemeOptions),
}

impl From<CustomTheme> for ThemeKind {
    fn from(custom_theme: CustomTheme) -> ThemeKind {
        if custom_theme.name.as_str().starts_with("Base16") {
            ThemeKind::CustomBase16(custom_theme)
        } else {
            ThemeKind::Custom(custom_theme)
        }
    }
}

impl std::fmt::Display for ThemeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match &self {
            ThemeKind::Light => "Light",
            ThemeKind::Dark => "Dark",
            ThemeKind::Dracula => "Dracula",
            ThemeKind::SolarizedDark => "Solarized Dark",
            ThemeKind::SolarizedLight => "Solarized Light",
            ThemeKind::GruvboxDark => "Gruvbox Dark",
            ThemeKind::GruvboxLight => "Gruvbox Light",
            ThemeKind::JellyFish => "Jellyfish",
            ThemeKind::Koi => "Koi",
            ThemeKind::Leafy => "Leafy",
            ThemeKind::Marble => "Marble",
            ThemeKind::PinkCity => "Pink City",
            ThemeKind::Snowy => "Snowy",
            ThemeKind::DarkCity => "Dark City",
            ThemeKind::RedRock => "Red Rock",
            ThemeKind::CyberWave => "Cyber Wave",
            ThemeKind::WillowDream => "Willow Dream",
            ThemeKind::FancyDracula => "Fancy Dracula",
            ThemeKind::Phenomenon => "Phenomenon",
            ThemeKind::SolarFlare => "Solar Flare",
            ThemeKind::Adeberry => "Adeberry",
            ThemeKind::SentReferralReward => "Warp Referral",
            ThemeKind::ReceivedReferralReward => "Referred to Warp",
            ThemeKind::Custom(custom_theme) => custom_theme.name.as_str(),
            ThemeKind::CustomBase16(custom_theme) => custom_theme.name.as_str(),
            ThemeKind::InMemory(in_memory_theme) => in_memory_theme.name.as_str(),
        };
        write!(f, "{value}")
    }
}

impl ThemeKind {
    pub fn matches(&self, query: &str) -> bool {
        let theme_name = format!("{self}").to_lowercase();
        theme_name.contains(&query.to_lowercase())
    }
}

#[derive(
    Debug,
    Clone,
    Hash,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    PartialOrd,
    Ord,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "A user-provided custom theme.")]
pub struct CustomTheme {
    #[schemars(description = "The display name of the custom theme.")]
    name: String,
    #[schemars(description = "The file path to the custom theme definition.")]
    path: PathBuf,
}

impl CustomTheme {
    pub fn new(s: String, p: PathBuf) -> Self {
        CustomTheme { name: s, path: p }
    }

    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }
}

#[derive(
    Debug,
    Clone,
    Hash,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    PartialOrd,
    Ord,
    settings_value::SettingsValue,
)]
pub struct InMemoryThemeOptions {
    name: String,
    path: PathBuf,
    #[serde(skip)]
    possible_bg_colors: Vec<ColorU>,
    #[serde(skip)]
    chosen_bg_color_index: usize,
}

impl InMemoryThemeOptions {
    pub async fn new(name: String, path: PathBuf) -> Result<Self> {
        top_colors_for_image(path.clone()).map(|top_colors| InMemoryThemeOptions {
            name,
            path,
            possible_bg_colors: top_colors,
            chosen_bg_color_index: 0,
        })
    }

    pub fn chosen_bg_color(&self) -> ColorU {
        self.possible_bg_colors[self.chosen_bg_color_index]
    }

    pub fn possible_bg_colors(&self) -> Vec<ColorU> {
        self.possible_bg_colors.clone()
    }

    pub fn chosen_bg_color_index(&self) -> usize {
        self.chosen_bg_color_index
    }

    pub fn set_chosen_bg_color_index(&mut self, index: usize) {
        self.chosen_bg_color_index = index;
    }

    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.path = path;
    }

    pub fn theme(&self) -> WarpTheme {
        let bg_color = self.chosen_bg_color();
        let fg_color = pick_foreground_color(bg_color);
        let possible_accent_colors: Vec<ColorU> = self
            .possible_bg_colors
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != self.chosen_bg_color_index)
            .map(|(_, color)| *color)
            .collect();
        let accent_color =
            pick_accent_color_from_options(&[bg_color, fg_color], &possible_accent_colors[..]);

        let (details, terminal_colors) = if fg_color.eq(&ColorU::white()) {
            (Details::Darker, dark_mode_colors())
        } else {
            (Details::Lighter, light_mode_colors())
        };

        WarpTheme::new(
            bg_color.into(),
            fg_color,
            accent_color.into(),
            None,
            Some(details),
            terminal_colors,
            Some(Image {
                // Note that, as an invariant, in-memory themes come from local files.
                source: AssetSource::LocalFile {
                    path: self.path().to_str().unwrap_or_default().to_owned(),
                },
                opacity: 30,
            }),
            Some(self.name()),
        )
    }
}

#[derive(Debug, Clone)]
pub struct WarpThemeConfig {
    theme_map: HashMap<ThemeKind, WarpTheme>,
}

impl WarpThemeConfig {
    pub fn new() -> Self {
        // preload with built-in themes
        let theme_map: HashMap<ThemeKind, WarpTheme> = HashMap::from_iter([
            (ThemeKind::SentReferralReward, sent_referral_reward()),
            (
                ThemeKind::ReceivedReferralReward,
                received_referral_reward(),
            ),
            (ThemeKind::Dark, dark_theme()),
            (ThemeKind::Light, light_theme()),
            (ThemeKind::SolarizedDark, solarized_dark()),
            (ThemeKind::SolarizedLight, solarized_light()),
            (ThemeKind::Dracula, dracula()),
            (ThemeKind::GruvboxDark, gruvbox_dark()),
            (ThemeKind::GruvboxLight, gruvbox_light()),
            (ThemeKind::JellyFish, jellyfish()),
            (ThemeKind::Koi, koi()),
            (ThemeKind::Leafy, leafy()),
            (ThemeKind::Marble, marble()),
            (ThemeKind::PinkCity, pink_city()),
            (ThemeKind::Snowy, snowy()),
            (ThemeKind::DarkCity, dark_city()),
            (ThemeKind::RedRock, red_rock()),
            (ThemeKind::CyberWave, cyber_wave()),
            (ThemeKind::WillowDream, willow_dream()),
            (ThemeKind::FancyDracula, fancy_dracula()),
            (ThemeKind::Phenomenon, phenomenon()),
            (ThemeKind::SolarFlare, solar_flare()),
            (ThemeKind::Adeberry, adeberry()),
        ]);
        WarpThemeConfig { theme_map }
    }

    pub fn add_new_theme(&mut self, theme_name: ThemeKind, theme: WarpTheme) {
        self.theme_map.insert(theme_name, theme);
    }

    pub fn file_to_theme(name: String, path: PathBuf) -> ThemeKind {
        CustomTheme::new(name, path).into()
    }

    pub fn theme_items(&self) -> impl Iterator<Item = (&ThemeKind, &WarpTheme)> {
        self.theme_map.iter()
    }

    pub fn theme(&self, name: &ThemeKind) -> WarpTheme {
        self.theme_map.get(name).cloned().unwrap_or_else(dark_theme)
    }
}

impl Default for WarpThemeConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RespectSystemTheme {
    #[default]
    Off,
    On(SelectedSystemThemes),
}

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Themes to use when following the system light/dark mode.")]
pub struct SelectedSystemThemes {
    #[schemars(description = "The theme to use in light mode.")]
    pub light: ThemeKind,
    #[schemars(description = "The theme to use in dark mode.")]
    pub dark: ThemeKind,
}

impl RespectSystemTheme {
    pub fn selected_system_themes(&self) -> Option<&SelectedSystemThemes> {
        match self {
            RespectSystemTheme::Off => None,
            RespectSystemTheme::On(selected) => Some(selected),
        }
    }
}

impl Default for SelectedSystemThemes {
    fn default() -> Self {
        Self {
            light: ThemeKind::Light,
            dark: ThemeKind::Dark,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct PromptColors {
    pub input_prompt_conversation_management: ColorU,
    pub input_prompt_pwd: ColorU,
    pub input_prompt_git: ColorU,
    pub input_prompt_branch: ColorU,
    pub input_prompt_agent_mode_hint: ColorU,
    pub input_prompt_agent_mode_tasks: ColorU,
    pub input_prompt_dirty_color: ColorU,
    pub input_prompt_virtual_env: ColorU,
    pub input_prompt_user_and_host: ColorU,
    pub input_prompt_date: ColorU,
    pub input_prompt_time: ColorU,
    pub input_prompt_kubernetes: ColorU,
    pub input_prompt_svn: ColorU,
    pub input_prompt_separator: ColorU,
    pub input_prompt_subshell: ColorU,
    pub input_prompt_ssh: ColorU,
}

impl From<WarpTheme> for PromptColors {
    fn from(theme: WarpTheme) -> Self {
        PromptColors {
            input_prompt_conversation_management: theme.terminal_colors().normal.white.into(),
            input_prompt_pwd: theme.terminal_colors().normal.magenta.into(),
            input_prompt_git: theme.terminal_colors().normal.green.into(),
            input_prompt_agent_mode_hint: theme.terminal_colors().normal.yellow.into(),
            input_prompt_agent_mode_tasks: theme.terminal_colors().normal.yellow.into(),
            input_prompt_branch: theme.terminal_colors().normal.yellow.into(),
            input_prompt_dirty_color: theme.terminal_colors().normal.green.into(),
            input_prompt_virtual_env: theme.terminal_colors().normal.yellow.into(),
            input_prompt_user_and_host: theme.terminal_colors().normal.green.into(),
            input_prompt_date: theme.terminal_colors().normal.cyan.into(),
            input_prompt_time: theme.terminal_colors().normal.red.into(),
            input_prompt_kubernetes: theme.terminal_colors().normal.cyan.into(),
            input_prompt_ssh: theme.terminal_colors().normal.blue.into(),
            input_prompt_subshell: theme.terminal_colors().normal.blue.into(),
            input_prompt_svn: theme.terminal_colors().normal.blue.into(),
            input_prompt_separator: theme.terminal_colors().normal.magenta.into(),
        }
    }
}

pub fn render_preview(
    theme: &WarpTheme,
    font_family: FamilyId,
    form_factor: Option<f32>,
) -> Box<dyn Element> {
    let text_size = 8. * form_factor.unwrap_or(1.);
    let margin = THUMBNAIL_MARGIN * form_factor.unwrap_or(1.);
    let padding = 5. * form_factor.unwrap_or(1.);
    let text_line_1 = Container::new(
        Text::new_inline("ls", font_family, text_size)
            .with_color(theme.foreground().into_solid())
            .finish(),
    )
    .with_margin_left(margin)
    .with_margin_right(margin)
    .finish();

    let text_line_2 = Container::new(
        Flex::row()
            .with_child(
                Text::new_inline("dir   ", font_family, text_size)
                    .with_color(theme.terminal_colors().normal.blue.into())
                    .finish(),
            )
            .with_child(
                Text::new_inline("executable   ", font_family, text_size)
                    .with_color(theme.terminal_colors().normal.red.into())
                    .finish(),
            )
            .with_child(
                Text::new_inline("file", font_family, text_size)
                    .with_color(theme.foreground().into_solid())
                    .finish(),
            )
            .finish(),
    )
    .with_margin_left(margin)
    .with_margin_right(margin)
    .finish();

    let input_box = Shrinkable::new(
        1.,
        Align::new(
            Flex::column()
                // The border above the input box.
                .with_child(
                    Container::new(Empty::new().finish())
                        .with_padding_bottom(padding)
                        .with_border(
                            Border::top(1.2 * form_factor.unwrap_or(1.))
                                .with_border_color(theme.outline().into_solid()),
                        )
                        .finish(),
                )
                // The fake cursor within the input box.
                .with_child(
                    Container::new(
                        ConstrainedBox::new(
                            Rect::new()
                                .with_background_color(theme.accent().into_solid())
                                .finish(),
                        )
                        .with_height(12. * form_factor.unwrap_or(1.))
                        .with_width(2. * form_factor.unwrap_or(1.))
                        .finish(),
                    )
                    .with_margin_left(margin)
                    .with_margin_right(margin)
                    .finish(),
                )
                .finish(),
        )
        .bottom_left()
        .finish(),
    )
    .finish();

    let mut thumbnail = Stack::new();
    let mut background_opacity = 100;
    if let Some(background_image) = theme.background_image() {
        thumbnail.add_child(
            Shrinkable::new(
                1.,
                warpui::elements::Image::new(
                    background_image.source(),
                    warpui::elements::CacheOption::BySize,
                )
                .cover()
                .finish(),
            )
            .finish(),
        );
        background_opacity -= background_image.opacity;
    }

    thumbnail.add_child(
        Container::new(
            Container::new(
                Flex::column()
                    .with_child(text_line_1)
                    .with_child(
                        Container::new(text_line_2)
                            .with_padding_top(padding)
                            .finish(),
                    )
                    .with_child(input_box)
                    .finish(),
            )
            .with_margin_top(margin)
            .with_margin_bottom(margin)
            .finish(),
        )
        .with_background(theme.background().with_opacity(background_opacity))
        .finish(),
    );

    Align::new(
        Container::new(
            ConstrainedBox::new(thumbnail.finish())
                .with_height(100. * form_factor.unwrap_or(1.))
                .with_width(190. * form_factor.unwrap_or(1.))
                .finish(),
        )
        .finish(),
    )
    .finish()
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
