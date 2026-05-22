use std::borrow::Cow;

use pathfinder_geometry::vector::vec2f;

use crate::elements::{
    Align, Border, ChildAnchor, ConstrainedBox, Container, CrossAxisAlignment, Element, Empty,
    Flex, Hoverable, Icon, MainAxisAlignment, MainAxisSize, MouseState, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Shrinkable, Stack,
};
use crate::geometry::vector::Vector2F;
use crate::platform::Cursor;
use crate::ui_components::components::{UiComponent, UiComponentStyles};
use crate::ui_components::text::Span;

/// Enum specifying relative alignment of the text and icon within
/// a button.  "First" is used instead of left/right to make this
/// robust to RTL languages.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextAndIconAlignment {
    /// Render the icon before the text.
    IconFirst,
    /// Render the text before the icon.
    TextFirst,
}

/// Configuration data for a button containing both a text and an icon.
#[derive(Clone)]
pub struct TextAndIcon {
    alignment: TextAndIconAlignment,
    /// The amount of space the `Flex` row should consume along the main axis.
    flex_size: MainAxisSize,
    /// The alignment strategy for rendering the `Flex`.
    flex_spacing: MainAxisAlignment,
    text: Cow<'static, str>,
    icon: Icon,
    /// Padding between the text and the icon.
    padding: f32,
    icon_size: Vector2F,
}

impl TextAndIcon {
    pub fn new(
        alignment: TextAndIconAlignment,
        text: impl Into<Cow<'static, str>>,
        icon: Icon,
        flex_size: MainAxisSize,
        flex_spacing: MainAxisAlignment,
        icon_size: Vector2F,
    ) -> Self {
        Self {
            alignment,
            flex_size,
            flex_spacing,
            text: text.into(),
            icon,
            padding: 0.,
            icon_size,
        }
    }

    pub fn with_inner_padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }
}

enum ButtonLabel {
    None,
    /// A start-aligned text label.
    Text(String),
    /// A center-aligned text label.
    CenteredText(String),
    Icon(Icon),
    TextAndIcon(TextAndIcon),
    Custom(Box<dyn Element>),
}

pub struct Button {
    label: ButtonLabel,
    /// Should the button be clickable?
    disabled: bool,
    /// Was the button clicked and its state is active?
    active: bool,
    styles: UiComponentStyles,
    /// Used when the button is hovered, if None - falls back to `styles`
    hovered_styles: Option<UiComponentStyles>,
    /// Used when the button is clicked, if None - falls back to `styles`
    clicked_styles: Option<UiComponentStyles>,
    /// Used when the button is disabled, if None - falls back to `styles`
    disabled_styles: Option<UiComponentStyles>,
    /// Used when the button is active, if None - falls back to `clicked_styles` when available,
    /// or `styles` otherwise
    active_styles: Option<UiComponentStyles>,
    render_tooltip_fn: Option<Box<dyn FnOnce() -> Box<dyn Element>>>,
    tooltip_position: ButtonTooltipPosition,
    hover_state: MouseStateHandle,
    cursor: Option<Cursor>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    Basic,
    Secondary,
    Accent,
    Outlined,
    Warn,
    Error,
    Text,
    Link,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonTooltipPosition {
    /// Position the tooltip above the button (center-aligned).
    Above,
    /// Position the tooltip below the button (center-aligned).
    #[default]
    Below,
    /// Position the tooltip above the button (left-aligned).
    AboveLeft,
    /// Position the tooltip above the button (right-aligned).
    AboveRight,
    /// Position the tooltip below the button (left-aligned).
    BelowLeft,
    /// Position the tooltip below the button (right-aligned).
    BelowRight,
}

impl UiComponent for Button {
    type ElementType = Hoverable;
    fn build(self) -> Hoverable {
        let disabled = self.disabled;
        let cursor = self.cursor;
        let mut hoverable = Hoverable::new(self.hover_state.clone(), |state| {
            self.render_inner_button(state)
        });
        if let Some(cursor) = cursor {
            hoverable = hoverable.with_cursor(cursor);
        }
        if disabled {
            return hoverable.disable();
        }
        hoverable
    }

    /// Overwrites _some_ styles passed in `style` parameter
    fn with_style(self, styles: UiComponentStyles) -> Self {
        Button {
            styles: self.styles.merge(styles),
            hovered_styles: Some(self.hovered_styles.unwrap_or(self.styles).merge(styles)),
            clicked_styles: Some(self.clicked_styles.unwrap_or(self.styles).merge(styles)),
            disabled_styles: Some(self.disabled_styles.unwrap_or(self.styles).merge(styles)),
            active_styles: Some(self.active_styles.unwrap_or(self.styles).merge(styles)),
            ..self
        }
    }
}

impl Button {
    pub fn new(
        mouse_state: MouseStateHandle,
        default_styles: UiComponentStyles,
        hovered_styles: Option<UiComponentStyles>,
        clicked_styles: Option<UiComponentStyles>,
        disabled_styles: Option<UiComponentStyles>,
    ) -> Self {
        Button {
            label: ButtonLabel::None,
            disabled: false,
            styles: default_styles,
            hovered_styles,
            clicked_styles,
            disabled_styles,
            active_styles: None,
            active: false,
            hover_state: mouse_state,
            render_tooltip_fn: None,
            tooltip_position: Default::default(),
            cursor: Some(Cursor::PointingHand),
        }
    }

    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    pub fn active(mut self) -> Self {
        self.active = true;
        self
    }

    pub fn with_text_label(mut self, label: String) -> Self {
        self.label = ButtonLabel::Text(label);
        self
    }

    pub fn with_centered_text_label(mut self, label: String) -> Self {
        self.label = ButtonLabel::CenteredText(label);
        self
    }

    pub fn with_icon_label(mut self, icon: Icon) -> Self {
        self.label = ButtonLabel::Icon(icon);
        self
    }

    pub fn with_cursor(mut self, cursor: Option<Cursor>) -> Self {
        self.cursor = cursor;
        self
    }

    pub fn with_active_styles(mut self, styles: UiComponentStyles) -> Self {
        self.active_styles = Some(self.styles.merge(styles));
        self
    }

    pub fn with_hovered_styles(mut self, styles: UiComponentStyles) -> Self {
        self.hovered_styles = Some(self.styles.merge(styles));
        self
    }

    pub fn hovered_styles(&self) -> &UiComponentStyles {
        self.hovered_styles.as_ref().unwrap_or(&self.styles)
    }

    pub fn with_disabled_styles(mut self, styles: UiComponentStyles) -> Self {
        self.disabled_styles = Some(self.styles.merge(styles));
        self
    }

    pub fn with_clicked_styles(mut self, styles: UiComponentStyles) -> Self {
        self.clicked_styles = Some(self.styles.merge(styles));
        self
    }

    /// Renders text followed by an icon within the Button.
    pub fn with_text_and_icon_label(mut self, text_and_icon: TextAndIcon) -> Self {
        self.label = ButtonLabel::TextAndIcon(text_and_icon);
        self
    }

    pub fn with_custom_label(mut self, label: Box<dyn Element>) -> Self {
        self.label = ButtonLabel::Custom(label);
        self
    }

    pub fn with_tooltip<F>(mut self, render_tooltip_fn: F) -> Self
    where
        F: 'static + FnOnce() -> Box<dyn Element>,
    {
        self.render_tooltip_fn = Some(Box::new(render_tooltip_fn));
        self
    }

    /// Sets how the tooltip is positioned relative to the button itself. This only has an effect
    /// if a tooltip is set with [`Self::with_tooltip`].
    pub fn with_tooltip_position(mut self, position: ButtonTooltipPosition) -> Self {
        self.tooltip_position = position;
        self
    }

    fn styles(&self, state: &MouseState) -> UiComponentStyles {
        // disabled button ignores click/hover events
        if self.disabled {
            return self.disabled_styles.unwrap_or(self.styles);
        }

        if self.active {
            return self
                .active_styles
                .unwrap_or_else(|| self.clicked_styles.unwrap_or(self.styles));
        }

        // For hover styles, we want to show the correct style based on
        // where the mouse _currently_ is, rather than whether the element
        // is considered hovered, because the latter takes into account delays.
        if state.is_mouse_over_element() {
            if state.is_clicked() {
                return self.clicked_styles.unwrap_or(self.styles);
            }
            return self.hovered_styles.unwrap_or(self.styles);
        }
        self.styles
    }

    fn render_inner_button(mut self, state: &MouseState) -> Box<dyn Element> {
        let styles = self.styles(state);
        // Text & font / Icon
        let label = match self.label {
            ButtonLabel::Text(text) => Span::new(text, styles).build().finish(),
            ButtonLabel::CenteredText(text) => {
                Align::new(Span::new(text, styles).build().finish()).finish()
            }
            ButtonLabel::Icon(icon) => {
                if let Some(color) = styles.font_color {
                    icon.with_color(color).finish()
                } else {
                    icon.finish()
                }
            }
            ButtonLabel::TextAndIcon(text_and_icon) => {
                let text = Shrinkable::new(
                    1.,
                    Container::new(Span::new(text_and_icon.text, styles).build().finish()).finish(),
                )
                .finish();
                let icon = if let Some(color) = styles.font_color {
                    text_and_icon.icon.with_color(color).finish()
                } else {
                    text_and_icon.icon.finish()
                };
                let icon = ConstrainedBox::new(icon)
                    .with_width(text_and_icon.icon_size.x())
                    .with_height(text_and_icon.icon_size.y())
                    .finish();

                let (first, second) = if text_and_icon.alignment == TextAndIconAlignment::TextFirst
                {
                    (text, icon)
                } else {
                    (icon, text)
                };

                Flex::row()
                    .with_children([
                        first,
                        Container::new(second)
                            .with_padding_left(text_and_icon.padding)
                            .finish(),
                    ])
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_alignment(text_and_icon.flex_spacing)
                    .with_main_axis_size(text_and_icon.flex_size)
                    .finish()
            }
            ButtonLabel::Custom(element) => element,
            ButtonLabel::None => Empty::new().finish(),
        };

        let mut container = Container::new(label);
        // Setting up the border
        if let Some(corner) = styles.border_radius {
            container = container.with_corner_radius(corner);
        }
        // TODO border width separate for top/left/right/bottom
        let mut border = Border::all(styles.border_width.unwrap_or_default());
        if let Some(border_color) = styles.border_color {
            border = border.with_border_fill(border_color);
        }
        container = container.with_border(border);

        // Position-related settings
        if let Some(padding) = styles.padding {
            container = container
                .with_padding_left(padding.left)
                .with_padding_top(padding.top)
                .with_padding_right(padding.right)
                .with_padding_bottom(padding.bottom);
        }
        if let Some(margin) = styles.margin {
            container = container
                .with_margin_left(margin.left)
                .with_margin_top(margin.top)
                .with_margin_right(margin.right)
                .with_margin_bottom(margin.bottom);
        }

        if let Some(background) = styles.background {
            container = container.with_background(background);
        }

        let container = match (styles.height, styles.width) {
            (None, None) => container.finish(),
            (_, _) => {
                let mut constrained_box = ConstrainedBox::new(container.finish());
                if let Some(height) = styles.height {
                    constrained_box = constrained_box.with_height(height);
                }
                if let Some(width) = styles.width {
                    constrained_box = constrained_box.with_width(width);
                }
                constrained_box.finish()
            }
        };

        // The tooltip should only be shown if the element
        // is considered hovered (accounting for delays).
        if state.is_hovered() {
            if let Some(render_tooltip_fn) = self.render_tooltip_fn.take() {
                // Keep stack within this rather than using a stack for all cases to allow multiple stack overlays to work
                let mut stack = Stack::new();
                stack.add_child(container);
                let tooltip = render_tooltip_fn();
                let tooltip_offset = match self.tooltip_position {
                    ButtonTooltipPosition::Above => OffsetPositioning::offset_from_parent(
                        vec2f(0., -8.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopMiddle,
                        ChildAnchor::BottomMiddle,
                    ),
                    ButtonTooltipPosition::Below => OffsetPositioning::offset_from_parent(
                        vec2f(0., 8.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomMiddle,
                        ChildAnchor::TopMiddle,
                    ),
                    ButtonTooltipPosition::AboveLeft => OffsetPositioning::offset_from_parent(
                        vec2f(0., -8.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    ),
                    ButtonTooltipPosition::BelowLeft => OffsetPositioning::offset_from_parent(
                        vec2f(0., 8.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomLeft,
                        ChildAnchor::TopLeft,
                    ),
                    ButtonTooltipPosition::AboveRight => OffsetPositioning::offset_from_parent(
                        vec2f(0., -8.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopRight,
                        ChildAnchor::BottomRight,
                    ),
                    ButtonTooltipPosition::BelowRight => OffsetPositioning::offset_from_parent(
                        vec2f(0., 8.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomRight,
                        ChildAnchor::TopRight,
                    ),
                };
                stack.add_positioned_overlay_child(tooltip, tooltip_offset);
                return stack.finish();
            }
        }

        container
    }

    pub fn set_clicked_styles(mut self, styles: Option<UiComponentStyles>) -> Self {
        self.clicked_styles = styles;
        self
    }
}
