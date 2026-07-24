//! Shared types for message bar rendering across terminal and agent views.
pub mod attached_context;
pub mod common;

use std::borrow::Cow;
use std::sync::Arc;

use pathfinder_color::ColorU;
use warp_core::ui::Icon;
use warpui::EventContext;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::MouseStateHandle;
use warpui::keymap::Keystroke;
/// A trait for types that can produce a message based on some contextual arguments.
///
/// The generic `Args` parameter allows each call site to define its own
/// context-specific arguments (e.g., terminal-specific or agent-specific state).
pub trait MessageProvider<Args> {
    /// Produces a message based on the given arguments, or `None` if this
    /// provider does not apply in the current context.
    fn produce_message(&self, args: Args) -> Option<Message>;
}

/// A trait for types that can transform an existing message based on some contextual arguments.
pub trait MessageTransformer<Args> {
    /// Modifies the given message.
    ///
    /// Returns `true` if the message was indeed modified.
    fn transform_message(&self, message: &mut Message, args: Args) -> bool;
}

/// A message to be displayed in a message bar, composed of a sequence of items.
#[derive(Default, Clone)]
pub struct Message {
    /// The items that make up this message.
    pub items: Vec<MessageItem>,
}

impl Message {
    /// Creates a new message with the given items.
    pub fn new(items: Vec<MessageItem>) -> Self {
        Self { items }
    }

    /// Creates a message from a single text string.
    pub fn from_text(text: impl Into<Cow<'static, str>>) -> Self {
        Self {
            items: vec![MessageItem::text(text)],
        }
    }

    /// Appends text to the last item if it's a Text item, otherwise adds a new text item.
    pub fn append_text(&mut self, text: &str) {
        if let Some(MessageItem::Text { content, .. }) = self.items.last_mut() {
            *content = Cow::Owned(format!("{}{}", content, text));
        } else {
            self.items.push(MessageItem::text(text.to_owned()));
        }
    }

    /// Sets the color override for all text items in the message.
    pub fn with_text_color(mut self, color: ColorU) -> Self {
        for item in &mut self.items {
            item.set_text_color(color);
        }
        self
    }

    /// Sets the color override for all items in the message.
    pub fn with_color(mut self, color: ColorU) -> Self {
        self.set_color(color);
        self
    }

    /// Sets the color override for all items in the message.
    pub fn set_color(&mut self, color: ColorU) {
        for item in &mut self.items {
            item.set_color(color);
        }
    }

    pub fn take_items(self) -> Vec<MessageItem> {
        self.items
    }
}

pub type ClickHandler = Arc<dyn Fn(&mut EventContext) + 'static>;

/// Horizontal alignment for chip items in the message bar.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum ChipHorizontalAlignment {
    /// Align the chip to the left (default behavior).
    #[default]
    Left,
    /// Align the chip to the right of the message bar.
    Right,
}

/// An item within a message, either a keystroke, text, icon, or interactive group.
#[derive(Clone)]
pub enum MessageItem {
    /// A keyboard shortcut/keystroke to display.
    Keystroke {
        /// The keystroke to display.
        keystroke: Keystroke,
        /// Optional color override for this keystroke.
        color: Option<ColorU>,
        /// Optional background color override for this keystroke,
        background_color: Option<ColorU>,
    },
    /// Text content with an optional color override.
    Text {
        /// The text content.
        content: Cow<'static, str>,
        /// Optional color override for this text.
        color: Option<ColorU>,
    },
    /// A hyperlink that opens a URL.
    Hyperlink {
        /// The link text.
        content: Cow<'static, str>,
        /// The target URL.
        url: String,
        /// Optional color override for this link.
        color: Option<ColorU>,
        /// Mouse state handle for hover/click tracking.
        mouse_state: MouseStateHandle,
    },
    /// An icon with an optional color override.
    Icon {
        /// The icon to display.
        icon: Icon,
        /// Optional color override for this icon.
        color: Option<ColorU>,
    },
    /// A clickable group of items.
    Clickable {
        /// The items in this clickable group.
        items: Vec<MessageItem>,
        /// Click handler for the group.
        action: ClickHandler,
        /// Mouse state handle for hover/click tracking.
        mouse_state: MouseStateHandle,
        /// Whether this clickable is disabled (no hover/click behavior).
        disabled: bool,
    },
    /// A clickable chip group of items with container styling handled by renderers.
    Chip {
        /// The items rendered inside this chip.
        items: Vec<MessageItem>,
        /// Click handler for the chip.
        action: ClickHandler,
        /// Mouse state handle for hover/click tracking.
        mouse_state: MouseStateHandle,
        /// Whether this chip is disabled (no hover/click behavior).
        disabled: bool,
        /// Horizontal alignment of the chip within the message bar.
        horizontal_alignment: ChipHorizontalAlignment,
    },
    /// A raw image rendered from an asset source at a specific size.
    /// Unlike `Icon`, this preserves the original colors of the asset (e.g. colored SVGs).
    Image {
        /// The asset source for the image.
        source: AssetSource,
        /// Display width in logical pixels.
        width: f32,
        /// Display height in logical pixels.
        height: f32,
    },
}

impl MessageItem {
    /// Creates a keystroke item from a `Keystroke`.
    pub fn keystroke(keystroke: Keystroke) -> Self {
        Self::Keystroke {
            keystroke,
            color: None,
            background_color: None,
        }
    }

    /// Creates a text item with no color override.
    pub fn text(content: impl Into<Cow<'static, str>>) -> Self {
        Self::Text {
            content: content.into(),
            color: None,
        }
    }

    /// Creates a hyperlink item.
    pub fn hyperlink(
        content: impl Into<Cow<'static, str>>,
        url: String,
        mouse_state: MouseStateHandle,
    ) -> Self {
        Self::Hyperlink {
            content: content.into(),
            url,
            color: None,
            mouse_state,
        }
    }

    /// Creates an icon item with no color override.
    pub fn icon(icon: Icon) -> Self {
        Self::Icon { icon, color: None }
    }

    /// Creates an image item from an asset source at the given size.
    pub fn image(source: AssetSource, width: f32, height: f32) -> Self {
        Self::Image {
            source,
            width,
            height,
        }
    }

    /// Creates a clickable group of items.
    pub fn clickable(
        items: Vec<MessageItem>,
        action: impl Fn(&mut EventContext) + 'static,
        mouse_state: MouseStateHandle,
    ) -> Self {
        debug_assert!(
            !items.iter().any(|i| matches!(i, Self::Clickable { .. })),
            "Nested clickable message items are not supported"
        );
        Self::Clickable {
            items,
            action: Arc::new(action),
            mouse_state,
            disabled: false,
        }
    }

    /// Creates a clickable chip group of items.
    pub fn chip(
        items: Vec<MessageItem>,
        action: impl Fn(&mut EventContext) + 'static,
        mouse_state: MouseStateHandle,
    ) -> Self {
        debug_assert!(
            !items
                .iter()
                .any(|i| matches!(i, Self::Clickable { .. } | Self::Chip { .. })),
            "Nested interactive message items are not supported"
        );
        Self::Chip {
            items,
            action: Arc::new(action),
            mouse_state,
            disabled: false,
            horizontal_alignment: ChipHorizontalAlignment::default(),
        }
    }
    /// Sets the color for text items.
    pub fn set_text_color(&mut self, color: ColorU) {
        match self {
            Self::Text {
                color: item_color, ..
            }
            | Self::Hyperlink {
                color: item_color, ..
            } => {
                *item_color = Some(color);
            }
            Self::Clickable { items, .. } | Self::Chip { items, .. } => {
                for item in items {
                    item.set_text_color(color);
                }
            }
            Self::Keystroke { .. } | Self::Icon { .. } | Self::Image { .. } => {}
        }
    }

    /// Sets the color for all items.
    pub fn set_color(&mut self, color: ColorU) {
        match self {
            Self::Text {
                color: item_color, ..
            }
            | Self::Hyperlink {
                color: item_color, ..
            }
            | Self::Keystroke {
                color: item_color, ..
            } => {
                *item_color = Some(color);
            }
            Self::Clickable { items, .. } | Self::Chip { items, .. } => {
                for item in items {
                    item.set_color(color);
                }
            }
            Self::Icon {
                color: item_color, ..
            } => {
                *item_color = Some(color);
            }
            Self::Image { .. } => {}
        }
    }

    /// Sets disabled state for clickable items.
    /// This only affects interactivity (hover/click behavior).
    /// For visual styling, use `set_color()` or `set_text_color()`.
    pub fn set_is_disabled(&mut self, is_disabled: bool) {
        match self {
            Self::Clickable {
                disabled, items, ..
            }
            | Self::Chip {
                disabled, items, ..
            } => {
                *disabled = is_disabled;
                for item in items {
                    item.set_is_disabled(is_disabled);
                }
            }
            _ => {}
        }
    }

    /// Sets disabled state for clickable items and returns self.
    pub fn with_is_disabled(mut self, is_disabled: bool) -> Self {
        self.set_is_disabled(is_disabled);
        self
    }

    /// Sets the horizontal alignment for chip items and returns self.
    /// Has no effect on non-chip items.
    pub fn with_horizontal_alignment(mut self, alignment: ChipHorizontalAlignment) -> Self {
        if let Self::Chip {
            horizontal_alignment,
            ..
        } = &mut self
        {
            *horizontal_alignment = alignment;
        }
        self
    }
}

pub struct EmptyMessageProducer;

impl<T> MessageProvider<T> for EmptyMessageProducer {
    fn produce_message(&self, _: T) -> Option<Message> {
        Some(Message::from_text(""))
    }
}

use crate::util::truncation::truncate_from_end;

/// Returns a truncated command string for display in message bars.
/// Limits to 27 characters (including ellipsis) if truncated.
pub fn truncated_command_for_block(command: &str) -> String {
    truncate_from_end(command, 27)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
