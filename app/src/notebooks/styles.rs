//! Shared styles for notebooks.

use warpui::elements::{
    Container, CrossAxisAlignment, Flex, Hoverable, MainAxisAlignment, MouseStateHandle,
    ParentElement,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent as _;
use warpui::units::{IntoPixels, Pixels};
use warpui::{fonts, Element};

use crate::appearance::Appearance;
use crate::settings::{derived_notebook_font_size, FontSettings};
use crate::themes::theme::Fill;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;

/// Multiplier of the notebook font size for the title. At the default size, this
/// is 20px.
const TITLE_FONT_MULTIPLIER: f32 = 1.4;

const EDITOR_MAX_WIDTH: f32 = 640.;

const TITLE_MARGIN: f32 = 16.;
const EDITOR_PADDING_LEFT: f32 = 4.;
const EDITOR_PADDING_TOP: f32 = 4.;

/// Font size for the notebook title.
pub fn title_font_size(font_settings: &FontSettings) -> f32 {
    derived_notebook_font_size(font_settings) * TITLE_FONT_MULTIPLIER
}

/// Font properties for the notebook title.
pub const TITLE_FONT_PROPERTIES: fonts::Properties = fonts::Properties {
    style: fonts::Style::Normal,
    weight: fonts::Weight::Bold,
};

/// Wraps the title element in spacing. If not `None`, the details element is shown above the title.
pub fn wrap_title(title: Box<dyn Element>, details: Option<Box<dyn Element>>) -> Box<dyn Element> {
    let mut contents = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_main_axis_alignment(MainAxisAlignment::Center);

    if let Some(details) = details {
        contents.add_child(Container::new(details).with_padding_bottom(4.).finish())
    };

    contents.add_child(title);

    Container::new(contents.finish())
        .with_uniform_margin(TITLE_MARGIN)
        .finish()
}

/// Wraps the body element in spacing.
pub fn wrap_body(body: Box<dyn Element>) -> Box<dyn Element> {
    Container::new(body)
        .with_padding_left(EDITOR_PADDING_LEFT)
        .with_padding_top(EDITOR_PADDING_TOP)
        .finish()
}

/// The color to use for title/heading text.
pub fn title_text_fill(appearance: &Appearance) -> Fill {
    let theme = appearance.theme();
    theme.sub_text_color(theme.background())
}

/// Builds an action button for a block's footer (such as the button to run a command or embedded
/// workflow).
pub(super) fn block_footer_action_button(
    appearance: &Appearance,
    icon: Icon,
    mouse_state_handle: MouseStateHandle,
    tooltip: impl Into<String> + 'static,
    keybinding: Option<String>,
) -> Hoverable {
    let tooltip_builder = appearance.ui_builder().clone();
    icon_button(appearance, icon, false, mouse_state_handle)
        .with_tooltip(move || match keybinding {
            Some(keybinding) => tooltip_builder
                .tool_tip_with_sublabel(tooltip.into(), keybinding)
                .build()
                .finish(),
            None => tooltip_builder.tool_tip(tooltip.into()).build().finish(),
        })
        .build()
        // Revert to the default cursor instead of the editor I-beam
        .with_cursor(Cursor::Arrow)
}

// Maximum notebook editor width.
pub fn notebook_editor_max_width() -> Pixels {
    EDITOR_MAX_WIDTH.into_pixels()
}
