use std::borrow::Cow;
use std::sync::Arc;

use warp_core::ui::Icon;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss, Element, Flex,
    ParentElement, Radius, Shrinkable,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::Keystroke;
use warpui::prelude::{MainAxisAlignment, MainAxisSize, Text};
use warpui::{AppContext, EventContext};

use crate::{Component, Options as _, Renderable, button};

const CORNER_RADIUS: Radius = Radius::Pixels(8.);
const BORDER_WIDTH: f32 = 1.;

/// Base unit of dialog padding. The dialog component applies this to the header, but consumers are responsible
/// for adding their own content padding. This allows for full-width contents, such as dividers, which should
/// not have any padding.
pub const BASE_PADDING: f32 = 12.;

/// Horizontal padding that consumers should apply to their contents.
pub const HORIZONTAL_PADDING: f32 = 2. * BASE_PADDING;

/// A reusable dialog component that renders content in a styled container.
#[derive(Default)]
pub struct Dialog {
    close_button: button::Button,
}

pub struct Params<'a> {
    /// Dialog title.
    pub title: Cow<'static, str>,
    /// The content to display inside the dialog.
    pub content: Box<dyn Renderable<'a>>,
    /// Optional configuration for the dialog.
    pub options: Options<'a>,
}

impl<'a> crate::Params for Params<'a> {
    type Options<'o> = Options<'o>;
}

/// A function that handles dismiss events.
pub type DismissHandler = Arc<dyn Fn(&mut EventContext, &AppContext)>;

pub struct Options<'a> {
    /// Optional fixed width for the dialog. If not set, the dialog will size to its content.
    pub width: Option<f32>,

    /// Handler to invoke when the dialog is dismissed.
    /// If `None`, the dialog is not dismissible.
    pub on_dismiss: Option<DismissHandler>,

    /// Optional keystroke associated with the dismiss action. This will be rendered alongside
    /// the dismiss button in the dialog, but the caller is responsible for adding a keybinding.
    pub dismiss_keystroke: Option<Keystroke>,

    /// Optional footer to display at the bottom of the dialog.
    pub footer: Option<Box<dyn Renderable<'a>>>,
}

impl<'a> crate::Options for Options<'a> {
    fn default(_appearance: &Appearance) -> Self {
        Self {
            width: None,
            on_dismiss: None,
            dismiss_keystroke: None,
            footer: None,
        }
    }
}

impl Component for Dialog {
    type Params<'a> = Params<'a>;

    fn render<'a>(&self, appearance: &Appearance, params: Self::Params<'a>) -> Box<dyn Element> {
        let theme = appearance.theme();
        let options = params.options;

        let background = theme.surface_1();
        let text_color = theme.main_text_color(background).into_solid();
        let border_color = internal_colors::neutral_4(theme);

        let mut header_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        header_row.add_child(
            Shrinkable::new(
                1.,
                Text::new_inline(
                    params.title,
                    appearance.ui_font_family(),
                    appearance.header_font_size(),
                )
                .with_color(text_color)
                .with_style(Properties::default().weight(Weight::Semibold))
                .finish(),
            )
            .finish(),
        );

        if let Some(on_dismiss) = options.on_dismiss.clone() {
            let close_button = self.close_button.render(
                appearance,
                button::Params {
                    content: button::Content::Icon(Icon::X),
                    theme: &button::themes::Naked,
                    options: button::Options {
                        keystroke: options.dismiss_keystroke.clone(),
                        on_click: Some(Box::new(move |ctx, app, _| on_dismiss(ctx, app))),
                        ..button::Options::default(appearance)
                    },
                },
            );
            header_row.add_child(close_button);
        }

        let header = Container::new(header_row.finish())
            .with_horizontal_padding(HORIZONTAL_PADDING)
            .with_padding_top(2. * BASE_PADDING)
            .with_padding_bottom(BASE_PADDING)
            .finish();

        let footer = options.footer.map(|footer| {
            Container::new(footer.render(appearance))
                .with_vertical_padding(BASE_PADDING)
                .with_horizontal_padding(HORIZONTAL_PADDING)
                .with_border(Border::top(BORDER_WIDTH).with_border_color(border_color))
                .finish()
        });

        let body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(params.content.render(appearance))
            .with_children(footer)
            .finish();

        let container = Container::new(body)
            .with_background(background)
            .with_corner_radius(CornerRadius::with_all(CORNER_RADIUS))
            .with_border(Border::all(BORDER_WIDTH).with_border_color(border_color))
            .finish();

        let sized_container = if let Some(width) = options.width {
            ConstrainedBox::new(container).with_width(width).finish()
        } else {
            container
        };

        if let Some(on_dismiss) = options.on_dismiss {
            Dismiss::new(sized_container)
                .prevent_interaction_with_other_elements()
                .on_dismiss(move |ctx, app| on_dismiss(ctx, app))
                .finish()
        } else {
            sized_container
        }
    }
}
