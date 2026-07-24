use pathfinder_geometry::vector::vec2f;

use super::components::{UiComponent, UiComponentStyles};
use super::text::Span;
use crate::Element;
use crate::elements::{
    ChildAnchor, ConstrainedBox, Container, Empty, Hoverable, MouseState, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Stack,
};
use crate::scene::Border;

/// A button element used to toggle a single value on or off.
pub struct ToggleButton {
    label: ToggleButtonLabel,
    tooltip: Option<Box<dyn Element>>,
    toggled_on: bool,
    mouse_state: MouseStateHandle,
    styles: UiComponentStyles,
    hovered_styles: Option<UiComponentStyles>,
    toggled_on_styles: Option<UiComponentStyles>,
}

pub enum ToggleButtonLabel {
    None,
    Text(String),
}

impl<S> From<S> for ToggleButtonLabel
where
    S: Into<String>,
{
    fn from(label: S) -> Self {
        Self::Text(label.into())
    }
}

impl ToggleButton {
    pub fn new(mouse_state: MouseStateHandle, styles: UiComponentStyles) -> Self {
        Self {
            label: ToggleButtonLabel::None,
            toggled_on: false,
            tooltip: None,
            mouse_state,
            styles,
            hovered_styles: None,
            toggled_on_styles: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<ToggleButtonLabel>) -> Self {
        self.label = label.into();
        self
    }

    pub fn with_tooltip(mut self, tooltip: Box<dyn Element>) -> Self {
        self.tooltip = Some(tooltip);
        self
    }

    pub fn with_toggled_on(mut self, toggled_on: bool) -> Self {
        self.toggled_on = toggled_on;
        self
    }

    pub fn with_hovered_styles(mut self, styles: UiComponentStyles) -> Self {
        self.hovered_styles = Some(styles);
        self
    }

    pub fn with_toggled_on_styles(mut self, styles: UiComponentStyles) -> Self {
        self.toggled_on_styles = Some(styles);
        self
    }

    fn styles(&self, state: &MouseState) -> UiComponentStyles {
        let mut styles = self.styles;
        if self.toggled_on
            && let Some(overlay) = self.toggled_on_styles
        {
            styles = styles.merge(overlay);
        }

        if state.is_mouse_over_element()
            && let Some(overlay) = self.hovered_styles
        {
            styles = styles.merge(overlay);
        }
        styles
    }

    fn render_button(&self, styles: &UiComponentStyles) -> Box<dyn Element> {
        let label = match &self.label {
            ToggleButtonLabel::Text(text) => Span::new(text.clone(), *styles).build().finish(),
            ToggleButtonLabel::None => Empty::new().finish(),
        };

        let mut constrained_box = ConstrainedBox::new(label);
        if let Some(width) = styles.width {
            constrained_box = constrained_box.with_width(width);
        }
        if let Some(height) = styles.height {
            constrained_box = constrained_box.with_height(height);
        };

        let mut button = Container::new(constrained_box.finish());

        if let Some(background) = styles.background {
            button = button.with_background(background);
        }

        if let Some(corner_radius) = styles.border_radius {
            button = button.with_corner_radius(corner_radius);
        }

        if let Some(padding) = styles.padding {
            button = button
                .with_padding_top(padding.top)
                .with_padding_bottom(padding.bottom)
                .with_padding_left(padding.left)
                .with_padding_right(padding.right);
        }

        let mut border = Border::all(styles.border_width.unwrap_or_default());
        if let Some(border_fill) = styles.border_color {
            border = border.with_border_fill(border_fill);
        }
        button = button.with_border(border);

        button.finish()
    }
}

impl UiComponent for ToggleButton {
    type ElementType = Hoverable;

    fn build(mut self) -> Hoverable {
        Hoverable::new(self.mouse_state.clone(), |state| {
            let styles = self.styles(state);
            let button = self.render_button(&styles);
            let mut stack = Stack::new().with_child(button);

            if state.is_hovered()
                && let Some(tooltip) = self.tooltip.take()
            {
                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., 10.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::BottomRight,
                        ChildAnchor::TopRight,
                    ),
                )
            }

            stack.finish()
        })
    }

    fn with_style(mut self, style: UiComponentStyles) -> Self {
        self.styles = style;
        self
    }
}
