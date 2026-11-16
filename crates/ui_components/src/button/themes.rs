use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::theme::Fill;
use warp_core::ui::theme::color::internal_colors;

/// Theming delegate for a button.
pub trait Theme {
    /// The background fill for the button.
    fn background(&self, button_state: super::State, appearance: &Appearance) -> Option<Fill>;

    /// The color to use for text and icons, given the current background color.
    fn text_color(&self, background: Option<Fill>, appearance: &Appearance) -> ColorU;

    /// The border color for the button, if any.
    fn border(&self, _: &Appearance) -> Option<ColorU> {
        None
    }

    /// The border color for the keyboard shortcut, if any.
    fn keyboard_shortcut_border(&self, _text_color: ColorU, _: &Appearance) -> Option<ColorU> {
        None
    }

    /// The background color for the keyboard shortcut, if any.
    fn keyboard_shortcut_background(&self, _: &Appearance) -> Option<ColorU> {
        None
    }
}

/// "Primary" buttons have a colorful fill.
///
/// [Figma spec](https://www.figma.com/design/chk9pwt35jTJhf9KnHmZyE/Components?node-id=3628-14344&t=GRYXipD0INVmDupA-0)
pub struct Primary;

impl Theme for Primary {
    fn background(&self, button_state: super::State, appearance: &Appearance) -> Option<Fill> {
        match button_state {
            super::State::Default => Some(appearance.theme().accent()),
            super::State::Hovered => Some(internal_colors::accent_overlay_4(appearance.theme())),
            super::State::Pressed => Some(internal_colors::accent_overlay_3(appearance.theme())),
        }
    }

    fn text_color(&self, background: Option<Fill>, appearance: &Appearance) -> ColorU {
        let theme = appearance.theme();
        let bg = background.unwrap_or_else(|| theme.accent()).into_solid();
        theme.font_color(bg).into_solid()
    }

    fn keyboard_shortcut_border(&self, text_color: ColorU, _: &Appearance) -> Option<ColorU> {
        Some(coloru_with_opacity(text_color, 60))
    }
}

/// "Secondary" buttons have no fill and a border.
///
/// [Figma spec](https://www.figma.com/design/chk9pwt35jTJhf9KnHmZyE/Components?node-id=3628-14344&t=L1sS5Nxu1zzpWPYp-0)
pub struct Secondary;

impl Theme for Secondary {
    fn background(&self, button_state: super::State, appearance: &Appearance) -> Option<Fill> {
        match button_state {
            super::State::Default => None,
            super::State::Hovered => Some(internal_colors::fg_overlay_2(appearance.theme())),
            super::State::Pressed => Some(internal_colors::fg_overlay_3(appearance.theme())),
        }
    }

    fn text_color(&self, _background: Option<Fill>, appearance: &Appearance) -> ColorU {
        appearance.theme().foreground().into()
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(internal_colors::neutral_4(appearance.theme()))
    }

    fn keyboard_shortcut_background(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(internal_colors::neutral_3(appearance.theme()))
    }
}

/// "Disabled" buttons have a disabled fill and text color.
///
/// [Figma spec](https://www.figma.com/design/chk9pwt35jTJhf9KnHmZyE/Components?node-id=3628-14344&t=c27DwGHWevMlisVN-0)
pub struct Disabled;

impl Theme for Disabled {
    fn background(&self, _button_state: super::State, _: &Appearance) -> Option<Fill> {
        None
    }

    fn text_color(&self, _background: Option<Fill>, appearance: &Appearance) -> ColorU {
        internal_colors::neutral_5(appearance.theme())
    }

    fn keyboard_shortcut_background(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(internal_colors::neutral_3(appearance.theme()))
    }
}

/// "Naked" buttons have no fill or border by default, only their contents.
///
/// This is typically used for link-like or text-style actions.
pub struct Naked;

impl Theme for Naked {
    fn background(&self, button_state: super::State, appearance: &Appearance) -> Option<Fill> {
        match button_state {
            super::State::Default => None,
            super::State::Hovered => Some(internal_colors::fg_overlay_2(appearance.theme())),
            super::State::Pressed => Some(internal_colors::fg_overlay_3(appearance.theme())),
        }
    }

    fn text_color(&self, _background: Option<Fill>, appearance: &Appearance) -> ColorU {
        appearance.theme().foreground().into_solid()
    }

    fn keyboard_shortcut_background(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(internal_colors::neutral_3(appearance.theme()))
    }
}
