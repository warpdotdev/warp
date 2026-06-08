use std::borrow::Cow;

use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui_core::keymap::Keystroke;
use warpui_core::prelude::*;

use crate::{Component, keyboard_shortcut};

/// Use a smaller-than-normal font size for the tooltip text to make it more compact.
const UI_FONT_SIZE_ADJUSTMENT: f32 = -2.;

#[derive(Default)]
pub struct Tooltip;

pub struct Params {
    pub label: Cow<'static, str>,
    pub options: Options,
}

impl crate::Params for Params {
    type Options<'a> = Options;
}

pub struct Options {
    pub keyboard_shortcut: Option<Keystroke>,
}

impl crate::Options for Options {
    fn default(_: &Appearance) -> Self {
        Self {
            keyboard_shortcut: None,
        }
    }
}

impl Component for Tooltip {
    type Params<'a> = Params;

    fn render<'a>(
        &self,
        appearance: &Appearance,
        params: Self::Params<'a>,
    ) -> Box<dyn warpui_core::Element> {
        let font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size() + UI_FONT_SIZE_ADJUSTMENT;

        let mut content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(10.)
            .with_child(
                Text::new(params.label, font_family, font_size)
                    .soft_wrap(false)
                    .with_color(appearance.theme().background().into_solid())
                    .finish(),
            );

        if let Some(keystroke) = params.options.keyboard_shortcut {
            content.add_child(keyboard_shortcut::KeyboardShortcut.render(
                appearance,
                keyboard_shortcut::Params {
                    keystroke,
                    options: keyboard_shortcut::Options {
                        font_color: Some(internal_colors::semantic_text_disabled(
                            appearance.theme(),
                        )),
                        sizing: keyboard_shortcut::Sizing {
                            font_size,
                            ..crate::Options::default(appearance)
                        },
                        ..crate::Options::default(appearance)
                    },
                },
            ));
        }

        Container::new(content.finish())
            .with_horizontal_padding(7.)
            .with_vertical_padding(3.)
            .with_background(appearance.theme().tooltip_background())
            .with_border(Border::all(1.).with_border_fill(appearance.theme().surface_2()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish()
    }
}
