use std::borrow::Cow;

use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warpui::keymap::Keystroke;
use warpui::platform::OperatingSystem;
use warpui::prelude::*;

use crate::Component;

#[derive(Default)]
pub struct KeyboardShortcut;

pub struct Params {
    pub keystroke: Keystroke,
    pub options: Options,
}

impl crate::Params for Params {
    type Options<'a> = Options;
}

pub struct Options {
    pub font_color: Option<ColorU>,
    pub background: Option<Fill>,
    pub border_fill: Option<Fill>,
    pub sizing: Sizing,
}

impl crate::Options for Options {
    fn default(appearance: &Appearance) -> Self {
        Self {
            font_color: None,
            background: None,
            border_fill: None,
            sizing: Sizing::default(appearance),
        }
    }
}

#[derive(Copy, Clone)]
pub struct Sizing {
    pub font_size: f32,
    pub padding: f32,
}

impl crate::Options for Sizing {
    fn default(appearance: &Appearance) -> Self {
        Self {
            font_size: appearance.ui_font_size() - 1.,
            padding: 4.,
        }
    }
}

impl Component for KeyboardShortcut {
    type Params<'a> = Params;

    fn render<'a>(&self, appearance: &Appearance, params: Self::Params<'a>) -> Box<dyn Element> {
        Flex::row()
            .with_spacing(4.)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children(
                keystroke_to_keys(params.keystroke)
                    .into_iter()
                    .map(|key| key.render(&params.options, appearance)),
            )
            .finish()
    }
}

fn keystroke_to_keys(keystroke: Keystroke) -> Vec<Key> {
    let mut keys = Vec::new();
    // Note: The order of the modifiers is intentional, to match the VS Code command palette
    if keystroke.ctrl {
        keys.push(Key::Control);
    }

    if keystroke.shift {
        keys.push(Key::Shift);
    }

    if keystroke.meta {
        keys.push(Key::Meta);
    }

    if keystroke.alt {
        keys.push(Key::Option);
    }

    if keystroke.cmd {
        keys.push(Key::Command);
    }

    keys.push(Key::Other(keystroke.key.into()));
    keys
}

#[derive(Clone)]
enum Key {
    Command,
    Option,
    Control,
    Shift,
    Meta,
    Other(Cow<'static, str>),
}

impl Key {
    fn text(&self, is_lowercase_modifier: bool) -> Cow<'static, str> {
        let is_mac = OperatingSystem::get().is_mac();
        let mut text: Cow<'static, str> = match self {
            Key::Command => if is_mac { "⌘" } else { "Logo" }.into(),
            Key::Option => if is_mac { "⌥" } else { "Alt" }.into(),
            Key::Control => if is_mac { "⌃" } else { "Ctrl" }.into(),
            Key::Shift => if is_mac { "⇧" } else { "Shift" }.into(),
            Key::Meta => "Meta".into(),
            Key::Other(key) => match key.as_ref() {
                "up" => "↑".into(),
                "down" => "↓".into(),
                "left" => "←".into(),
                "right" => "→".into(),
                "\t" => "Tab".into(),
                " " => "Space".into(),
                "escape" => "ESC".into(),
                "enter" => "⏎".into(),
                "delete" => "⌫".into(),
                _ => {
                    // Capitalize the first letter of the key name
                    key.chars()
                        .next()
                        .map(|c| c.to_ascii_uppercase())
                        .into_iter()
                        .chain(key.chars().skip(1))
                        .collect()
                }
            },
        };
        // Single character keys should still be uppercase.
        if text.len() > 1 && is_lowercase_modifier {
            text = text.to_lowercase().into();
        }
        text
    }

    fn render(&self, options: &Options, appearance: &Appearance) -> Box<dyn Element> {
        // TODO(vorporeal): consider supporting lowercase-only text
        let text = self.text(false);

        let font_size = options.sizing.font_size;
        let color = options
            .font_color
            .unwrap_or_else(|| appearance.theme().foreground().into());

        let content = if let Some(mut icon) = Icon::icon_for_key(text.as_ref()) {
            icon = icon.with_color(color);
            ConstrainedBox::new(icon.finish())
                .with_height(font_size)
                .with_width(font_size)
                .finish()
        } else {
            Text::new(text, appearance.ui_font_family(), font_size)
                .with_color(color)
                .with_line_height_ratio(1.)
                .with_selectable(false)
                .finish()
        };

        let is_naked = options.background.is_none() && options.border_fill.is_none();
        if is_naked {
            content
        } else {
            let border_width = 1.;
            let mut container = Container::new(
                // Ensure that the center alignment is applied properly if the content does not
                // meet the min width or height.
                MinSize::new(
                    Flex::row()
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(content)
                        .finish(),
                )
                .finish(),
            )
            .with_padding(Padding::uniform(options.sizing.padding))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)));
            if let Some(border_fill) = options.border_fill {
                container =
                    container.with_border(Border::all(border_width).with_border_fill(border_fill));
            }
            if let Some(background) = options.background {
                container = container.with_background(background);
            }

            // If there's some visual border (either due to a background or explicit border),
            // prevent a "tall rectangle" aspect ratio.
            let min_size = font_size + 2. * (options.sizing.padding + border_width);
            ConstrainedBox::new(container.finish())
                .with_min_height(min_size)
                .with_min_width(min_size)
                .finish()
        }
    }
}
