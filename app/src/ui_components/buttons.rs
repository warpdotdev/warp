use warpui::elements::{CornerRadius, MouseStateHandle, Radius};
use warpui::ui_components::button::Button;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};

use super::icons::{ICON_DIMENSIONS, Icon};
use super::{BORDER_RADIUS, blended_colors};
use crate::appearance::Appearance;
use crate::themes::theme::{Fill, WarpTheme};

const ICON_BUTTON_PADDING: f32 = 4.;

#[derive(Copy, Clone)]
enum ButtonMode {
    Base,
    #[allow(dead_code)]
    Accent,
}

#[derive(Copy, Clone)]
enum ButtonState {
    Default,
    Disabled,
    Pressed,
    Hover,
}

/// Utility struct that wraps all styles required for the button
pub struct AllButtonStyles {
    default_styles: UiComponentStyles,
    hovered_styles: Option<UiComponentStyles>,
    clicked_styles: Option<UiComponentStyles>,
    disabled_styles: Option<UiComponentStyles>,
}

fn all_icon_button_styles(warp_theme: &WarpTheme, mode: ButtonMode) -> AllButtonStyles {
    AllButtonStyles {
        default_styles: icon_button_styles(warp_theme, mode, ButtonState::Default),
        hovered_styles: Some(icon_button_styles(warp_theme, mode, ButtonState::Hover)),
        clicked_styles: Some(icon_button_styles(warp_theme, mode, ButtonState::Pressed)),
        disabled_styles: Some(icon_button_styles(warp_theme, mode, ButtonState::Disabled)),
    }
}

fn icon_button_styles(
    warp_theme: &WarpTheme,
    mode: ButtonMode,
    state: ButtonState,
) -> UiComponentStyles {
    let icon_color = icon_color(warp_theme, mode);

    let (background_color, border_color): (Option<Fill>, Option<Fill>) = match (mode, state) {
        (ButtonMode::Base, ButtonState::Default) => (None, None),
        (ButtonMode::Base, ButtonState::Hover) => {
            (Some(warp_theme.surface_2()), Some(warp_theme.surface_3()))
        }
        (ButtonMode::Base, ButtonState::Pressed) | (ButtonMode::Base, ButtonState::Disabled) => {
            (Some(warp_theme.background()), Some(warp_theme.surface_3()))
        }
        (ButtonMode::Accent, ButtonState::Default) => (None, None),
        (ButtonMode::Accent, ButtonState::Hover) => (
            Some(warp_theme.surface_3()),
            Some(blended_colors::accent(warp_theme)),
        ),
        (ButtonMode::Accent, ButtonState::Pressed)
        | (ButtonMode::Accent, ButtonState::Disabled) => (
            Some(warp_theme.background()),
            Some(blended_colors::accent_pressed(warp_theme)),
        ),
    };

    let mut styles = UiComponentStyles::default()
        .set_width(ICON_DIMENSIONS)
        .set_height(ICON_DIMENSIONS)
        .set_border_width(0.)
        .set_padding(Coords::uniform(ICON_BUTTON_PADDING))
        .set_border_radius(CornerRadius::with_all(Radius::Pixels(BORDER_RADIUS)))
        .set_font_color(icon_color.into());

    if let Some(border_color) = border_color {
        styles = styles.set_border_color(border_color.into());
    }
    if let Some(background_color) = background_color {
        styles = styles.set_background(background_color.into());
    }
    styles
}

fn combo_inner_button_styles(warp_theme: &WarpTheme, state: ButtonState) -> UiComponentStyles {
    let background = match state {
        ButtonState::Default => None,
        ButtonState::Hover => Some(blended_colors::neutral_2(warp_theme)),
        ButtonState::Pressed => Some(blended_colors::neutral_4(warp_theme)),
        ButtonState::Disabled => Some(warp_theme.background().into()),
    };

    UiComponentStyles {
        width: Some(ICON_DIMENSIONS),
        height: Some(ICON_DIMENSIONS),
        border_width: None,
        padding: Some(Coords::uniform(ICON_BUTTON_PADDING - 1.)),
        border_radius: None,
        font_color: Some(warp_theme.foreground().into()),
        border_color: None,
        background: background.map(Into::into),
        ..Default::default()
    }
}

/// This creates an inner icon_button for the purpose of adding it into a
/// combo button. In these cases, the icon_button should not have a border
/// as the combo button will provide these. Note that b/c
/// it is not needed at this time, disabled is not implemented.
///
/// TODO(CORE-2300): Evaluate whether or not this helper makes sense in this
/// location, as it is only used in workspace/view.rs right now (it is here
/// b/c of access to non-pub fields).
pub fn combo_inner_button(
    appearance: &Appearance,
    icon: Icon,
    active: bool,
    mouse_state_handle: MouseStateHandle,
) -> Button {
    let theme = appearance.theme();

    let button = Button::new(
        mouse_state_handle,
        combo_inner_button_styles(theme, ButtonState::Default),
        Some(combo_inner_button_styles(theme, ButtonState::Hover)),
        Some(combo_inner_button_styles(theme, ButtonState::Pressed)),
        Some(combo_inner_button_styles(theme, ButtonState::Disabled)),
    )
    .with_icon_label(icon.to_warpui_icon(theme.foreground()));

    if active {
        return button.active();
    }
    button
}

fn icon_color(warp_theme: &WarpTheme, mode: ButtonMode) -> Fill {
    match mode {
        ButtonMode::Base => warp_theme.foreground(),
        ButtonMode::Accent => blended_colors::accent(warp_theme),
    }
}

fn icon_button_internal(
    appearance: &Appearance,
    icon: Icon,
    active: bool,
    mouse_state_handle: MouseStateHandle,
    mode: ButtonMode,
    mut color: Option<Fill>,
) -> Button {
    let theme = appearance.theme();

    let button_styles = all_icon_button_styles(theme, mode);
    let mut button = Button::new(
        mouse_state_handle,
        button_styles.default_styles,
        button_styles.hovered_styles,
        button_styles.clicked_styles,
        button_styles.disabled_styles,
    )
    .with_icon_label(icon.to_warpui_icon(color.unwrap_or(icon_color(theme, mode))));

    if let Some(color) = color.take() {
        // We also need to set the font color here to get the button to be colored correctly.
        button = button.with_style(UiComponentStyles::default().set_font_color(color.into()));
    }

    if active {
        return button.active();
    }
    button
}

pub fn icon_button_with_color(
    appearance: &Appearance,
    icon: Icon,
    active: bool,
    mouse_state_handle: MouseStateHandle,
    color: Fill,
) -> Button {
    icon_button_internal(
        appearance,
        icon,
        active,
        mouse_state_handle,
        ButtonMode::Base,
        Some(color),
    )
}

pub fn icon_button(
    appearance: &Appearance,
    icon: Icon,
    active: bool,
    mouse_state_handle: MouseStateHandle,
) -> Button {
    icon_button_internal(
        appearance,
        icon,
        active,
        mouse_state_handle,
        ButtonMode::Base,
        None,
    )
}

pub fn accent_icon_button(
    appearance: &Appearance,
    icon: Icon,
    active: bool,
    mouse_state_handle: MouseStateHandle,
) -> Button {
    icon_button_internal(
        appearance,
        icon,
        active,
        mouse_state_handle,
        ButtonMode::Accent,
        None,
    )
}

pub fn close_button(appearance: &Appearance, mouse_state_handle: MouseStateHandle) -> Button {
    icon_button(appearance, Icon::X, false, mouse_state_handle)
}

pub fn highlight(button: Button, appearance: &Appearance) -> Button {
    button
        .with_style(UiComponentStyles::default().set_font_color(
            crate::ui_components::blended_colors::text_main(
                appearance.theme(),
                appearance.theme().background(),
            ),
        ))
        .with_hovered_styles(
            UiComponentStyles::default()
                .set_background(appearance.theme().surface_3().into())
                .set_font_color(appearance.theme().foreground().into()),
        )
        .with_clicked_styles(
            UiComponentStyles::default()
                .set_background(appearance.theme().background().into())
                .set_font_color(appearance.theme().foreground().into()),
        )
}
