use std::borrow::Cow;
use std::sync::Arc;

use itertools::Itertools;

use super::components::{UiComponent, UiComponentStyles};
use super::text::Span;
use crate::elements::{
    Align, ConstrainedBox, Container, CrossAxisAlignment, Element, Flex, Icon, MinSize,
    ParentElement, DEFAULT_UI_LINE_HEIGHT_RATIO,
};
use crate::keymap::Keystroke;
use crate::platform::OperatingSystem;
use crate::scene::Border;

type IconForKeystrokeFn = Arc<dyn Fn(&str) -> Option<Icon>>;

/// UI Component representing a keyboard shortcut, can be styled using `UiComponent::with_style`
#[derive(Clone)]
pub struct KeyboardShortcut {
    keys: Vec<Key>,
    style: UiComponentStyles,
    is_lowercase_modifier: bool,
    is_text_only: bool,
    space_between_keys: f32,
    line_height_ratio: f32,
    icon_for_keystroke: IconForKeystrokeFn,
}

impl KeyboardShortcut {
    pub fn new(keystroke: &Keystroke, style: UiComponentStyles) -> Self {
        Self {
            keys: keystroke_to_keys(keystroke),
            style,
            is_lowercase_modifier: false,
            is_text_only: false,
            space_between_keys: 3.,
            line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
            icon_for_keystroke: Arc::new(|_| None),
        }
    }

    pub fn lowercase_modifier(mut self) -> Self {
        self.is_lowercase_modifier = true;
        self
    }

    pub fn text_only(mut self) -> Self {
        self.is_text_only = true;
        self
    }

    pub fn with_space_between_keys(mut self, spacing: f32) -> Self {
        self.space_between_keys = spacing;
        self
    }

    pub fn with_line_height_ratio(mut self, line_height_ratio: f32) -> Self {
        self.line_height_ratio = line_height_ratio;
        self
    }

    pub fn with_icon_for_keystroke(
        mut self,
        icon_for_keystroke: impl Fn(&str) -> Option<Icon> + 'static,
    ) -> Self {
        self.icon_for_keystroke = Arc::new(icon_for_keystroke);
        self
    }
}

impl UiComponent for KeyboardShortcut {
    type ElementType = Container;

    fn build(self) -> Container {
        let keys = if self.is_text_only {
            // On Mac, we use symbols for modifiers so we don't need a separator.
            // On other OS, we spell out modifiers so they need to be separated by space
            let sep = if OperatingSystem::get().is_mac() {
                ""
            } else {
                " "
            };
            let combined_text = self
                .keys
                .iter()
                .map(|key| key.text(self.is_lowercase_modifier))
                .join(sep);

            let text_element = Align::new(
                Span::new(
                    combined_text,
                    // Removing any margin from the style passed to Span, since we process it below
                    self.style,
                )
                .with_line_height_ratio(self.line_height_ratio)
                .with_selectable(false)
                .build()
                .finish(),
            )
            .finish();
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(text_element)
        } else {
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children(self.keys.iter().enumerate().map(|(i, key)| {
                    if i == 0 {
                        key.render(
                            self.style,
                            self.is_lowercase_modifier,
                            self.line_height_ratio,
                            self.icon_for_keystroke.as_ref(),
                        )
                    } else {
                        Container::new(key.render(
                            self.style,
                            self.is_lowercase_modifier,
                            self.line_height_ratio,
                            self.icon_for_keystroke.as_ref(),
                        ))
                        .with_margin_left(self.space_between_keys)
                        .finish()
                    }
                }))
        };

        let mut keys = Container::new(keys.finish());

        if let Some(margin) = self.style.margin {
            keys = keys
                .with_margin_top(margin.top)
                .with_margin_right(margin.right)
                .with_margin_bottom(margin.bottom)
                .with_margin_left(margin.left);
        }

        keys
    }

    fn with_style(mut self, style: UiComponentStyles) -> Self {
        self.style = self.style.merge(style);
        self
    }
}

pub fn keystroke_to_keys(keystroke: &Keystroke) -> Vec<Key> {
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

    keys.push(Key::Other(keystroke.key.clone()));
    keys
}

#[derive(Clone)]
pub enum Key {
    Command,
    Option,
    Control,
    Shift,
    Meta,
    Other(String),
}

impl Key {
    pub fn text(&self, is_lowercase_modifier: bool) -> Cow<'static, str> {
        let mut text: Cow<'static, str> = match self {
            Key::Command => {
                if OperatingSystem::get().is_mac() {
                    "⌘".into()
                } else {
                    "Logo".into()
                }
            }
            Key::Option => {
                if OperatingSystem::get().is_mac() {
                    "⌥".into()
                } else {
                    "Alt".into()
                }
            }
            Key::Control => {
                if OperatingSystem::get().is_mac() {
                    "⌃".into()
                } else {
                    "Ctrl".into()
                }
            }
            Key::Shift => {
                if OperatingSystem::get().is_mac() {
                    "⇧".into()
                } else {
                    "Shift".into()
                }
            }
            Key::Meta => "Meta".into(),
            Key::Other(key) => match key.as_str() {
                "up" => "↑".into(),
                "down" => "↓".into(),
                "left" => "←".into(),
                "right" => "→".into(),
                "\t" => "Tab".into(),
                " " => "Space".into(),
                "escape" => "ESC".into(),
                "enter" => "⏎".into(),
                "backspace" => "⌫".into(),
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

    fn render(
        &self,
        style: UiComponentStyles,
        is_lowercase_modifier: bool,
        line_height_ratio: f32,
        icon_for_keystroke: &dyn Fn(&str) -> Option<Icon>,
    ) -> Box<dyn Element> {
        let text = self.text(is_lowercase_modifier);

        let (content, is_multi_char_key) = if let Some(mut icon) = icon_for_keystroke(text.as_ref())
        {
            if let Some(font_color) = style.font_color {
                icon = icon.with_color(font_color);
            }
            let size = style.font_size.unwrap_or_default();
            let icon = ConstrainedBox::new(icon.finish())
                .with_height(size)
                .with_width(size)
                .finish();
            (icon, false)
        } else {
            let is_multi_char_key = text.chars().count() > 1;
            let content = Span::new(
                text,
                // Removing any margin from the style passed to Span, since we process it below
                UiComponentStyles {
                    margin: None,
                    ..style
                },
            )
            .with_line_height_ratio(line_height_ratio)
            .with_selectable(false)
            .build()
            .finish();

            (content, is_multi_char_key)
        };

        let mut background = Container::new(MinSize::new(content).finish());

        let mut border = Border::all(style.border_width.unwrap_or_default());
        if let Some(border_color) = style.border_color {
            border = border.with_border_fill(border_color);
        }
        background = background.with_border(border);

        if let Some(padding) = style.padding {
            background = background
                .with_padding_top(padding.top)
                .with_padding_right(padding.right)
                .with_padding_bottom(padding.bottom)
                .with_padding_left(padding.left);
        }

        if is_multi_char_key
            && (style
                .padding
                .is_some_and(|padding| padding.left == 0. && padding.right == 0.)
                || style.padding.is_none())
        {
            // If this shortcut is for a keystroke represented with multiple chars and there is
            // no specified horizontal padding, add a default 4px horizontal padding. Because
            // it's multiple chars, itll exceed the given width constraint and leave you with a
            // shortcut with no padding.
            background = background.with_horizontal_padding(4.);
        }
        if let Some(radius) = style.border_radius {
            background = background.with_corner_radius(radius);
        }
        if let Some(background_color) = style.background {
            background = background.with_background(background_color);
        }

        let mut sized = ConstrainedBox::new(background.finish());
        match (style.width, style.height) {
            (Some(width), Some(height)) => {
                // If the height is set, use it as a minimum. If the content doesn't fill the
                // given height, grow each key to fit. If the content exceeds the given height,
                // allow it to grow to fit. This should not result in inconsistent heights since
                // each key will require the same amount of extra height (assuming all use the
                // same font, font size, and padding).
                // Allow the width to grow as needed to fit the content.
                sized = sized.with_min_width(width).with_min_height(height);
            }
            (None, Some(height)) => {
                // Make the minimum size a square as suggested by design if no width is given.
                sized = sized.with_min_width(height).with_min_height(height);
            }
            (Some(width), None) => {
                // Allow the width to grow as needed to fit the content.
                sized = sized.with_min_width(width);
            }
            (None, None) => (),
        }

        sized.finish()
    }
}
