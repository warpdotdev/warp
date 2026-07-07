//! This module contains rendering functions for various requested inline actions that have not yet
//! been transformed into a [`View`] component. This currently encompasses UI for file retrieval,
//! environmental variable collection, and SSH Warpification, to name a few.
//!
//! There's quite a bit of duplication between function-based inline actions and view-based inline
//! actions. Moreover, the header rendering functions here don't make use of the HeaderConfig.
//!
//! Ideally, the modules that currently use the functions herein should be transformed
//! into [`View`] components as well. If that's ever deemed necessary, see [`RequestedCommandView`]
//! for an example on how that transformation could be made.

use std::borrow::Cow;

use lazy_static::lazy_static;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors::neutral_2;
use warpui::elements::{
    Align, Border, Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex,
    FormattedTextElement, MainAxisAlignment, ParentElement, Radius, Shrinkable, Wrap, WrapFill,
};
use warpui::fonts::FamilyId;
use warpui::keymap::Keystroke;
use warpui::{AppContext, Element, SingletonEntity};

use super::inline_action_header::HeaderConfig;
use crate::ai::blocklist::block::view_impl::WithContentItemSpacing;
use crate::ai::blocklist::inline_action::inline_action_header;
use crate::ai::blocklist::inline_action::inline_action_header::{
    INLINE_ACTION_HEADER_VERTICAL_PADDING, INLINE_ACTION_HORIZONTAL_PADDING,
    INLINE_ACTION_VERTICAL_PADDING,
};
use crate::ai::blocklist::inline_action::inline_action_icons::icon_size;
use crate::ui_components::blended_colors;

lazy_static! {
    pub static ref ENTER_KEYSTROKE: Keystroke = Keystroke {
        key: "enter".to_owned(),
        ..Default::default()
    };
    pub static ref CMD_ENTER_KEYSTROKE: Keystroke =
        Keystroke::parse("cmdorctrl-enter").expect("RUN_REQUESTED_ACTION_KEYSTROKE is invalid");
    pub static ref CTRL_C_KEYSTROKE: Keystroke = Keystroke {
        ctrl: true,
        key: "c".to_owned(),
        ..Default::default()
    };
    pub static ref ESCAPE_KEYSTROKE: Keystroke = Keystroke {
        key: "escape".to_owned(),
        ..Default::default()
    };
}

pub(crate) enum FormattedTextOrElement {
    FormattedText(Box<FormattedTextElement>),
    Element(Box<dyn Element>),
}

impl From<FormattedTextElement> for FormattedTextOrElement {
    fn from(value: FormattedTextElement) -> Self {
        Self::FormattedText(Box::new(value))
    }
}

/// Configuration for rendering a requested action component using the builder pattern.
pub struct RenderableAction {
    body: FormattedTextOrElement,
    action_button: Option<Box<dyn Element>>,
    pub icon: Option<Box<dyn Element>>,
    pub header: Option<HeaderConfig>,
    pub footer: Option<Box<dyn Element>>,
    pub background_color: ColorU,
    pub should_highlight_border: bool,
    should_override_with_content_item_spacing: bool,
}

impl RenderableAction {
    pub fn new(text: &str, app: &AppContext) -> Self {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let formatted_text =
            render_requested_action_body_text(text.into(), appearance.ui_font_family(), app);
        Self {
            body: FormattedTextOrElement::FormattedText(Box::new(formatted_text)),
            icon: None,
            header: None,
            footer: None,
            action_button: None,
            background_color: neutral_2(theme),
            should_highlight_border: false,
            should_override_with_content_item_spacing: false,
        }
    }

    pub fn new_with_formatted_text(formatted_text: FormattedTextElement, app: &AppContext) -> Self {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Self {
            body: FormattedTextOrElement::FormattedText(Box::new(formatted_text)),
            icon: None,
            header: None,
            footer: None,
            action_button: None,
            background_color: neutral_2(theme),
            should_highlight_border: false,
            should_override_with_content_item_spacing: false,
        }
    }

    pub fn new_with_element(element: Box<dyn Element>, app: &AppContext) -> Self {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Self {
            body: FormattedTextOrElement::Element(element),
            icon: None,
            header: None,
            footer: None,
            action_button: None,
            background_color: neutral_2(theme),
            should_highlight_border: false,
            should_override_with_content_item_spacing: false,
        }
    }

    pub fn with_icon(mut self, icon: Box<dyn Element>) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn with_header(mut self, header: HeaderConfig) -> Self {
        self.header = Some(header);
        self
    }

    pub fn with_footer(mut self, footer: Box<dyn Element>) -> Self {
        self.footer = Some(footer);
        self
    }

    pub fn with_font_color(mut self, color: ColorU) -> Self {
        if let FormattedTextOrElement::FormattedText(formatted_text) = self.body {
            self.body =
                FormattedTextOrElement::FormattedText(Box::new(formatted_text.with_color(color)));
        }
        self
    }

    pub fn with_background_color(mut self, color: ColorU) -> Self {
        self.background_color = color;
        self
    }

    pub fn with_highlighted_border(mut self) -> Self {
        self.should_highlight_border = true;
        self
    }

    pub fn with_action_button(mut self, button: Box<dyn Element>) -> Self {
        self.action_button = Some(button);
        self
    }

    pub fn with_content_item_spacing(mut self) -> Self {
        self.should_override_with_content_item_spacing = true;
        self
    }

    /// Renders the requested action with the current configuration.
    pub fn render(self, app: &AppContext) -> Container {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut has_header = false;
        let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        if let Some(header) = self.header {
            content.add_child(Clipped::new(header.render(app)).finish());
            has_header = true;
        }

        content.add_child(render_requested_action_row(
            self.body,
            self.icon,
            self.action_button,
            true,
            has_header,
            app,
        ));

        if let Some(footer) = self.footer {
            content.add_child(
                Container::new(footer)
                    .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
                    .with_vertical_padding(4.)
                    .with_background(theme.surface_1())
                    .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
                    .finish(),
            );
        }

        let container = Container::new(content.finish())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_background_color(self.background_color)
            .with_border(
                Border::all(1.).with_border_fill(if self.should_highlight_border {
                    theme.accent()
                } else {
                    theme.surface_2()
                }),
            );

        if has_header || self.should_override_with_content_item_spacing {
            container.finish().with_content_item_spacing()
        } else {
            container.finish().with_agent_output_item_spacing(app)
        }
    }
}

pub fn render_requested_action_body_text(
    text: Cow<str>,
    font_family: FamilyId,
    app: &AppContext,
) -> FormattedTextElement {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    // Split text into lines and create FormattedTextLine for each
    let lines = text
        .lines()
        .map(|line| {
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text(line.to_owned())])
        })
        .collect::<Vec<_>>();

    let formatted_text = FormattedText::new(lines);
    FormattedTextElement::new(
        formatted_text.clone(),
        appearance.monospace_font_size(),
        font_family,
        font_family,
        blended_colors::text_main(theme, theme.background()),
        Default::default(),
    )
    .with_color(blended_colors::text_main(theme, theme.background()))
    .set_selectable(true)
}

/// Note that [`is_text_selectable`] is used to determine whether text selections are rendered.
/// A [`SelectableArea`] ancestor element is required to maintain functional text selection logic.
pub fn render_requested_action_row_for_text(
    text: Cow<str>,
    font_family: FamilyId,
    icon: Option<Box<dyn Element>>,
    acton_button: Option<Box<dyn Element>>,
    is_text_selectable: bool,
    has_header_above: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    render_requested_action_row(
        render_requested_action_body_text(text, font_family, app).into(),
        icon,
        acton_button,
        is_text_selectable,
        has_header_above,
        app,
    )
}

/// Renders a full-width, rounded rectangular row with the specified text and a custom icon.
/// Note that [`is_text_selectable`] is used to determine whether text selections are rendered.
/// A [`SelectableArea`] ancestor element is required to maintain functional text selection logic.
pub(crate) fn render_requested_action_row(
    text: FormattedTextOrElement,
    icon: Option<Box<dyn Element>>,
    action_button: Option<Box<dyn Element>>,
    is_text_selectable: bool,
    has_header_above: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let element = match text {
        FormattedTextOrElement::FormattedText(formatted_text) => {
            formatted_text.set_selectable(is_text_selectable).finish()
        }
        FormattedTextOrElement::Element(element) => element,
    };
    render_requested_action_row_for_element(element, icon, action_button, has_header_above, app)
}

/// Renders a full-width, rounded rectangular row with the specified text and a custom icon.
/// Note that [`is_text_selectable`] is used to determine whether text selections are rendered.
/// A [`SelectableArea`] ancestor element is required to maintain functional text selection logic.
fn render_requested_action_row_for_element(
    element: Box<dyn Element>,
    icon: Option<Box<dyn Element>>,
    action_button: Option<Box<dyn Element>>,
    has_header_above: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let has_action_button = action_button.is_some();

    let mut text_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    if let Some(icon) = icon {
        text_row.add_child(
            Container::new(
                ConstrainedBox::new(icon)
                    .with_width(icon_size(app))
                    .with_height(icon_size(app))
                    .finish(),
            )
            .with_margin_right(inline_action_header::ICON_MARGIN)
            .finish(),
        );
    }

    // When an action button is present we use a Wrap layout (below) so the button
    // flows to a second row on narrow panes.  In that case we must NOT wrap the text
    // in Align, because Align always reports the full constraint width, which would
    // inflate the text row and force the button to a new line unconditionally.
    if has_action_button {
        text_row.add_child(Shrinkable::new(1., element).finish());
    } else {
        text_row.add_child(Shrinkable::new(1., Align::new(element).left().finish()).finish());
    }

    let content = if let Some(action_button) = action_button {
        let button_element = Container::new(action_button)
            .with_margin_right(inline_action_header::ICON_MARGIN)
            .finish();
        let mut wrap = Wrap::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_run_spacing(8.);
        wrap.extend([
            WrapFill::new(0., text_row.finish()).finish(),
            button_element,
        ]);
        wrap.finish()
    } else {
        text_row.finish()
    };

    Container::new(content)
        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
        // The requested action row is currently overloaded, being used in two distinct ways:
        // 1) To display the body content of an inline requested action (with a header rendered above)
        // 2) To display the header of a non-expandable inline requested action
        .with_vertical_padding(if has_header_above {
            INLINE_ACTION_VERTICAL_PADDING
        } else {
            INLINE_ACTION_HEADER_VERTICAL_PADDING
        })
        .finish()
}
