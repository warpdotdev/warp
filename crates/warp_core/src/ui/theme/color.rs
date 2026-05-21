//! Module providing utility functions to retrieve the colors used within our ui system and
//! designs.
//! These colors can be further understood here:
//! https://docs.google.com/document/d/1YMovEoXsPRziPk99a4i9LZNEKGm_rjEyzhcHsFkT3ac/edit.

use self::internal_colors::{
    accent_overlay_2, fg_overlay_1, fg_overlay_2, fg_overlay_3, neutral_1, neutral_2, neutral_3,
    neutral_4,
};

use super::{AnsiColor, AnsiColorIdentifier, Fill, TerminalColors, WarpTheme};

use crate::ui::color::{
    blend::Blend,
    contrast::{pick_best_foreground_color, MinimumAllowedContrast},
    Opacity,
};
use getset::Getters;
use serde::{Deserialize, Serialize};
use warpui::color::ColorU;

const BLOCK_SELECTION_OPACITY: Opacity = 10;

#[derive(Serialize, Copy, Clone, Debug, Deserialize, Getters, PartialEq, Eq)]
#[get = "pub"]
// TODO handle optional fields (so users can specify some and not all)
pub struct CustomDetails {
    pub main_text_opacity: Opacity,
    pub sub_text_opacity: Opacity,
    pub hint_text_opacity: Opacity,
    pub disabled_text_opacity: Opacity,

    pub foreground_button_opacity: Opacity, // foreground color overlay on the std button
    pub accent_button_opacity: Opacity,     // foreground color overlay on the active button
    pub button_hover_opacity: Opacity,      // foreground color overlay on the button
    pub button_click_opacity: Opacity,      // bg color overlay on the button

    pub keybinding_row_overlay_opacity: Opacity,

    pub welcome_tips_completion_overlay_opacity: Opacity,
}

const DARKER_DETAILS: CustomDetails = CustomDetails {
    main_text_opacity: 90,
    sub_text_opacity: 60,
    hint_text_opacity: 40,
    disabled_text_opacity: 20,

    foreground_button_opacity: 30,
    accent_button_opacity: 0,
    button_hover_opacity: 10,
    button_click_opacity: 20,

    keybinding_row_overlay_opacity: 40,

    welcome_tips_completion_overlay_opacity: 90,
};

const LIGHTER_DETAILS: CustomDetails = CustomDetails {
    main_text_opacity: 90,
    sub_text_opacity: 60,
    hint_text_opacity: 40,
    disabled_text_opacity: 20,

    foreground_button_opacity: 30,
    accent_button_opacity: 0,
    button_hover_opacity: 10,
    button_click_opacity: 20,

    keybinding_row_overlay_opacity: 40,

    welcome_tips_completion_overlay_opacity: 90,
};

impl CustomDetails {
    pub fn darker_details() -> Self {
        DARKER_DETAILS
    }
    pub fn lighter_details() -> Self {
        LIGHTER_DETAILS
    }
}

impl Default for CustomDetails {
    fn default() -> Self {
        DARKER_DETAILS
    }
}

// Core colors
impl WarpTheme {
    pub fn accent(&self) -> Fill {
        self.accent
    }

    pub fn foreground(&self) -> Fill {
        Fill::Solid(self.foreground)
    }

    /// Background color for large backgrounds like that of the terminal view.
    /// Allows gradients because these are meant to be very large surfaces.
    pub fn background(&self) -> Fill {
        self.background
    }

    pub fn terminal_colors(&self) -> &TerminalColors {
        &self.terminal_colors
    }

    /// Background color for UI elements that need to stand out from the main
    /// `background()` color and the `surface_1()`` and `surface_2()`` backgrounds.
    /// Doesn't allow gradients because these surfaces will often be too small
    /// for the gradients to look appealing.
    pub fn surface_3(&self) -> Fill {
        Fill::Solid(neutral_3(self))
    }

    /// Background color for UI elements that need to stand out from the main
    /// `background()` color and the `surface_1()` color.
    /// Doesn't allow gradients because these surfaces will often be too small
    /// for the gradients to look appealing.
    pub fn surface_2(&self) -> Fill {
        Fill::Solid(neutral_2(self))
    }

    /// Background color for UI elements that need to stand out from the main
    /// `background()` color.
    /// Doesn't allow gradients because these surfaces will often be too small
    /// for the gradients to look appealing.
    pub fn surface_1(&self) -> Fill {
        Fill::Solid(neutral_1(self))
    }

    pub fn cursor(&self) -> Fill {
        self.cursor.unwrap_or(self.accent())
    }

    pub fn ui_warning_color(&self) -> ColorU {
        ColorU::new(194, 128, 0, 255)
    }

    pub fn ui_error_color(&self) -> ColorU {
        ColorU::new(188, 54, 42, 255)
    }

    pub fn ui_yellow_color(&self) -> ColorU {
        ColorU::new(229, 160, 26, 255)
    }

    pub fn ui_green_color(&self) -> ColorU {
        ColorU::new(28, 160, 90, 255)
    }

    pub fn outline(&self) -> Fill {
        fg_overlay_2(self)
    }

    // text colors
    pub fn font_color(&self, background: impl Into<ColorU>) -> Fill {
        Fill::Solid(pick_best_foreground_color(
            background.into(),
            self.background().into(),
            self.foreground().into(),
            MinimumAllowedContrast::Text,
        ))
    }

    pub fn main_text_color(&self, background: Fill) -> Fill {
        internal_colors::text_main(self, background).into()
    }

    pub fn sub_text_color(&self, background: Fill) -> Fill {
        internal_colors::text_sub(self, background).into()
    }

    pub fn hint_text_color(&self, background: Fill) -> Fill {
        let details = self.details();
        self.font_color(background)
            .with_opacity(details.hint_text_opacity)
    }

    pub fn disabled_text_color(&self, background: Fill) -> Fill {
        internal_colors::text_disabled(self, background).into()
    }

    pub fn active_ui_text_color(&self) -> Fill {
        self.main_text_color(self.surface_2())
    }

    pub fn nonactive_ui_text_color(&self) -> Fill {
        self.sub_text_color(self.surface_2())
    }

    pub fn disabled_ui_text_color(&self) -> Fill {
        self.disabled_text_color(self.surface_2())
    }

    pub fn active_highlighted_text_color(&self) -> Fill {
        self.main_text_color(self.accent())
    }

    pub fn settings_import_config_hover_opacity(&self) -> Opacity {
        10
    }

    pub fn dark_overlay(&self) -> Fill {
        let details = self.details();
        Fill::black().with_opacity(details.button_click_opacity)
    }

    pub fn keybinding_row_overlay(&self) -> Fill {
        let details = self.details();
        Fill::black().with_opacity(details.keybinding_row_overlay_opacity)
    }

    pub fn welcome_tips_completion_overlay(&self) -> Fill {
        let details = self.details();
        self.surface_2()
            .with_opacity(details.welcome_tips_completion_overlay_opacity)
    }

    pub fn blurred_background_overlay(&self) -> Fill {
        Fill::black().with_opacity(70)
    }
}

// Feature-specific theme colors
impl WarpTheme {
    pub fn foreground_button_color(&self) -> Fill {
        let details = self.details();
        self.background.blend(
            &self
                .foreground()
                .with_opacity(details.foreground_button_opacity),
        )
    }

    pub fn accent_button_color(&self) -> Fill {
        let details = self.details();
        self.accent.blend(
            &self
                .foreground()
                .with_opacity(details.accent_button_opacity),
        )
    }

    pub fn button_hover_opacity(&self, button: Fill) -> Fill {
        let details = self.details();
        button.blend(&self.foreground().with_opacity(details.button_hover_opacity))
    }

    pub fn split_pane_border_color(&self) -> Fill {
        fg_overlay_3(self)
    }

    pub fn accent_overlay(&self) -> Fill {
        accent_overlay_2(self)
    }

    pub fn surface_overlay_3(&self) -> Fill {
        fg_overlay_3(self)
    }

    pub fn surface_overlay_2(&self) -> Fill {
        fg_overlay_2(self)
    }

    pub fn surface_overlay_1(&self) -> Fill {
        fg_overlay_1(self)
    }

    pub fn yellow_overlay_1(&self) -> Fill {
        let yellow: Fill = self.ui_yellow_color().into();
        yellow.with_opacity(10)
    }

    pub fn green_overlay_1(&self) -> Fill {
        let green: Fill = self.ansi_fg_green().into();
        green.with_opacity(10)
    }

    pub fn green_overlay_2(&self) -> Fill {
        let green: Fill = self.ui_green_color().into();
        green.with_opacity(50)
    }

    pub fn block_selection_color(&self) -> Fill {
        accent_overlay_2(self)
    }

    pub fn block_selection_as_context_background_color(&self) -> Fill {
        let color_fill: Fill = self.terminal_colors.normal.yellow.into();
        color_fill.with_opacity(BLOCK_SELECTION_OPACITY)
    }

    pub fn block_selection_as_context_border_color(&self) -> Fill {
        let color_fill: Fill = self.terminal_colors.normal.yellow.into();
        color_fill
    }

    // Although text selection colors aren't yet themed, declaring them in this file
    // will make it easier to theme text selection colors in the future!
    pub fn text_selection_color(&self) -> Fill {
        Fill::Solid(ColorU::new(118, 167, 250, (0.4 * 255.) as u8))
    }

    pub fn text_selection_as_context_color(&self) -> Fill {
        self.ansi_overlay_2(self.terminal_colors.normal.yellow)
            .into()
    }

    pub fn find_bar_button_selection_color(&self) -> Fill {
        accent_overlay_2(self)
    }

    pub fn failed_block_color(&self) -> Fill {
        Fill::Solid(self.terminal_colors().normal.red.into())
    }

    pub fn active_ui_detail(&self) -> Fill {
        self.main_text_color(self.surface_2())
    }
    pub fn nonactive_ui_detail(&self) -> Fill {
        self.disabled_text_color(self.surface_2())
    }

    /// We apply an overlay over the terminal view background. The default overlay opacity is low
    /// so it doesn't conflict with window opacity adjustments.
    pub fn ai_blocks_overlay(&self) -> Fill {
        fg_overlay_1(self)
    }

    /// We apply an overlay over the terminal view background. The default overlay opacity is low
    /// so it doesn't conflict with window opacity adjustments.
    pub fn restored_blocks_overlay(&self) -> Fill {
        fg_overlay_2(self)
    }

    /// We apply an overlay over the terminal view background. The default overlay opacity is low
    /// so it doesn't conflict with window opacity adjustments.
    pub fn restored_ai_blocks_overlay(&self) -> Fill {
        fg_overlay_3(self)
    }

    pub fn inactive_pane_overlay(&self) -> Fill {
        fg_overlay_2(self)
    }

    pub fn subshell_background(&self) -> Fill {
        Fill::Solid(neutral_4(self))
    }

    pub fn block_banner_background(&self) -> Fill {
        Fill::Solid(neutral_3(self))
    }

    /// Background color for tooltips.
    /// Uses neutral_6 for better contrast with text.
    pub fn tooltip_background(&self) -> ColorU {
        internal_colors::neutral_6(self)
    }
}

// ANSI color blends
impl WarpTheme {
    pub fn ansi_bg(&self, ansi_color: AnsiColor) -> ColorU {
        let ansi_fill = Fill::from(ansi_color);
        self.background()
            .blend(&ansi_fill.with_opacity(50))
            .into_solid()
    }

    pub fn ansi_fg(&self, ansi_color: AnsiColor) -> ColorU {
        let ansi_fill = Fill::from(ansi_color);
        self.foreground()
            .blend(&ansi_fill.with_opacity(50))
            .into_solid()
    }

    pub fn ansi_overlay_1(&self, ansi_color: AnsiColor) -> ColorU {
        Fill::from(ansi_color).with_opacity(10).into_solid()
    }

    pub fn ansi_overlay_2(&self, ansi_color: AnsiColor) -> ColorU {
        Fill::from(ansi_color).with_opacity(50).into_solid()
    }

    pub fn ansi_fg_red(&self) -> ColorU {
        self.ansi_fg(AnsiColorIdentifier::Red.to_ansi_color(&self.terminal_colors().normal))
    }

    pub fn ansi_bg_red(&self) -> ColorU {
        self.ansi_bg(AnsiColorIdentifier::Red.to_ansi_color(&self.terminal_colors().normal))
    }

    pub fn ansi_fg_blue(&self) -> ColorU {
        self.ansi_fg(AnsiColorIdentifier::Blue.to_ansi_color(&self.terminal_colors().normal))
    }

    pub fn ansi_fg_green(&self) -> ColorU {
        self.ansi_fg(AnsiColorIdentifier::Green.to_ansi_color(&self.terminal_colors().normal))
    }

    pub fn ansi_bg_green(&self) -> ColorU {
        self.ansi_bg(AnsiColorIdentifier::Green.to_ansi_color(&self.terminal_colors().normal))
    }

    pub fn ansi_fg_yellow(&self) -> ColorU {
        self.ansi_fg(AnsiColorIdentifier::Yellow.to_ansi_color(&self.terminal_colors().normal))
    }

    pub fn ansi_bg_magenta(&self) -> ColorU {
        self.ansi_bg(AnsiColorIdentifier::Magenta.to_ansi_color(&self.terminal_colors().normal))
    }

    pub fn ansi_fg_magenta(&self) -> ColorU {
        self.ansi_fg(AnsiColorIdentifier::Magenta.to_ansi_color(&self.terminal_colors().normal))
    }

    pub fn ansi_bg_yellow(&self) -> ColorU {
        self.ansi_bg(AnsiColorIdentifier::Yellow.to_ansi_color(&self.terminal_colors().normal))
    }

    pub fn ansi_fg_cyan(&self) -> ColorU {
        self.ansi_fg(AnsiColorIdentifier::Cyan.to_ansi_color(&self.terminal_colors().normal))
    }
}

/// Internal color system tokens, defined in "Colors" [Figma project](https://www.figma.com/design/dnvTdLbfFaosFSP00F30S0/Colors).
/// Should not be used directly outside of reusable components. Use color methods on `WarpTheme` instead.
pub mod internal_colors {
    use warpui::color::ColorU;

    use super::{Fill, WarpTheme};
    use crate::ui::color::blend::Blend;
    use crate::ui::color::coloru_with_opacity;

    /// Calculates the font color based on contrast needs for text legibility.
    /// The font color is a mixture of the `warp_theme`'s background and foreground
    /// colors, and the supplied `background` color.
    fn font_color(warp_theme: &WarpTheme, background: impl Into<ColorU>) -> ColorU {
        warp_theme.font_color(background).into_solid()
    }

    /// Used for UI elements like buttons to which we want to call attention.
    /// Allows gradients so shouldn't be used for small elements.
    pub fn accent(warp_theme: &WarpTheme) -> Fill {
        warp_theme.accent()
    }

    /// Hover state for UI elements like buttons to which we want to call attention.
    /// Allows gradients so shouldn't be used for small elements.
    pub fn accent_hover(warp_theme: &WarpTheme) -> Fill {
        warp_theme
            .accent()
            .blend(&warp_theme.foreground().with_opacity(40))
    }

    /// Pressed state for UI elements like buttons
    /// to which we want to call attention.
    /// Allows gradients so shouldn't be used for small elements.
    #[allow(dead_code)]
    pub fn accent_pressed(warp_theme: &WarpTheme) -> Fill {
        warp_theme
            .accent()
            .blend(&warp_theme.background().with_opacity(30))
    }

    /// The color of most text throughout the UI.
    pub fn text_main(warp_theme: &WarpTheme, background: impl Into<ColorU>) -> ColorU {
        coloru_with_opacity(font_color(warp_theme, background), 90)
    }

    /// The color of subheaders and similar lower priority text.
    pub fn text_sub(warp_theme: &WarpTheme, background: impl Into<ColorU>) -> ColorU {
        coloru_with_opacity(font_color(warp_theme, background), 60)
    }

    /// The color of text elements that are disabled or the lowest priority.
    pub fn text_disabled(warp_theme: &WarpTheme, background: impl Into<ColorU>) -> ColorU {
        coloru_with_opacity(font_color(warp_theme, background), 40)
    }

    // TODO (roland): evaluate whether text_disabled above is intentionally different or if it should be consolidated with this
    // which matches figma mocks.
    pub fn semantic_text_disabled(warp_theme: &WarpTheme) -> ColorU {
        warp_theme
            .background()
            .blend(&fg_overlay_5(warp_theme))
            .into()
    }

    pub fn neutral_1(warp_theme: &WarpTheme) -> ColorU {
        warp_theme
            .background()
            .blend(&warp_theme.foreground().with_opacity(5))
            .into_solid()
    }

    pub fn neutral_2(warp_theme: &WarpTheme) -> ColorU {
        warp_theme
            .background()
            .blend(&warp_theme.foreground().with_opacity(10))
            .into_solid()
    }

    pub fn neutral_3(warp_theme: &WarpTheme) -> ColorU {
        warp_theme
            .background()
            .blend(&warp_theme.foreground().with_opacity(15))
            .into_solid()
    }

    pub fn neutral_4(warp_theme: &WarpTheme) -> ColorU {
        warp_theme
            .background()
            .blend(&warp_theme.foreground().with_opacity(20))
            .into_solid()
    }

    pub fn neutral_5(warp_theme: &WarpTheme) -> ColorU {
        warp_theme
            .background()
            .blend(&warp_theme.foreground().with_opacity(40))
            .into_solid()
    }

    pub fn neutral_6(warp_theme: &WarpTheme) -> ColorU {
        warp_theme
            .background()
            .blend(&warp_theme.foreground().with_opacity(60))
            .into_solid()
    }

    pub fn neutral_7(warp_theme: &WarpTheme) -> ColorU {
        warp_theme
            .background()
            .blend(&warp_theme.foreground().with_opacity(90))
            .into_solid()
    }

    pub fn fg_overlay_1(warp_theme: &WarpTheme) -> Fill {
        warp_theme.foreground().with_opacity(5)
    }

    pub fn fg_overlay_2(warp_theme: &WarpTheme) -> Fill {
        warp_theme.foreground().with_opacity(10)
    }

    pub fn fg_overlay_3(warp_theme: &WarpTheme) -> Fill {
        warp_theme.foreground().with_opacity(15)
    }

    pub fn fg_overlay_4(warp_theme: &WarpTheme) -> Fill {
        warp_theme.foreground().with_opacity(20)
    }

    pub fn fg_overlay_5(warp_theme: &WarpTheme) -> Fill {
        warp_theme.foreground().with_opacity(40)
    }

    pub fn fg_overlay_6(warp_theme: &WarpTheme) -> Fill {
        warp_theme.foreground().with_opacity(60)
    }

    pub fn fg_overlay_7(warp_theme: &WarpTheme) -> Fill {
        warp_theme.foreground().with_opacity(90)
    }

    pub fn accent_bg_strong(warp_theme: &WarpTheme) -> Fill {
        Fill::Solid(warp_theme.background().into_solid())
            .blend(&warp_theme.accent().with_opacity(60))
    }

    pub fn accent_bg(warp_theme: &WarpTheme) -> Fill {
        Fill::Solid(warp_theme.background().into_solid())
            .blend(&warp_theme.accent().with_opacity(40))
    }

    pub fn accent_fg_strong(warp_theme: &WarpTheme) -> Fill {
        warp_theme
            .foreground()
            .blend(&warp_theme.accent().with_opacity(60))
    }

    pub fn accent_fg(warp_theme: &WarpTheme) -> Fill {
        warp_theme
            .foreground()
            .blend(&warp_theme.accent().with_opacity(40))
    }

    pub fn accent_overlay_1(warp_theme: &WarpTheme) -> Fill {
        warp_theme.accent().with_opacity(10)
    }

    pub fn accent_overlay_2(warp_theme: &WarpTheme) -> Fill {
        warp_theme.accent().with_opacity(25)
    }

    pub fn accent_overlay_3(warp_theme: &WarpTheme) -> Fill {
        warp_theme.accent().with_opacity(40)
    }

    pub fn accent_overlay_4(warp_theme: &WarpTheme) -> Fill {
        warp_theme.accent().with_opacity(60)
    }
}
