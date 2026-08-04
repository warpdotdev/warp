use lazy_static::lazy_static;

use crate::color::ColorU;
use crate::elements::{
    AnchorPair, ChildAnchor, ConstrainedBox, Container, CornerRadius, Element, Empty, Fill, Flex,
    Hoverable, MouseState, MouseStateHandle, OffsetPositioning, OffsetType, ParentAnchor,
    ParentElement, ParentOffsetBounds, PositioningAxis, Rect, Stack, XAxisAnchor, YAxisAnchor,
};
use crate::geometry::vector::vec2f;
use crate::platform::Cursor;
use crate::scene::{DropShadow, Radius};
use crate::ui_components::components::{UiComponent, UiComponentStyles};
use crate::ui_components::text::Span;
use crate::ui_components::tool_tip::Tooltip;

const DEFAULT_THUMB_HEIGHT: f32 = 18.;

lazy_static! {
    // Hardcode for now, but can be made configurable if necessary.
    pub static ref TRACK_COLOR: ColorU = ColorU::new(170, 170, 170, 255);

    static ref DROP_SHADOW: DropShadow = DropShadow {
                            color: ColorU::black(),
                            offset: vec2f(-0.5, 2.),
                            blur_radius: 20.,
                            spread_radius: 0.,
                        };
}

/// A config to provide both the text and the styles for a tooltip.
/// Bundling these together prevents any callers from passing in just one
/// without the other (and this ui element is not capable of coming up with sensible, themed defaults for the tooltip styles).
#[derive(Clone)]
pub struct TooltipConfig {
    pub text: String,
    pub styles: UiComponentStyles,
}

/// A switch element used to toggle the on/off state of a single value. A switch consists of two
/// distinct pieces: the "thumb" which is the piece that is clickable and is rendered on the left if
/// unchecked and on the right if checked, and the "track", the background that the thumb moves
/// along. The switch optionally includes a label that can also be clicked to active the element.
/// Note the switch does not contain any state, it's up to the caller to rebuild the switch with the
/// correct value for "checked" when the switch is clicked.
pub struct Switch {
    checked: bool,
    disabled: bool,
    label: Option<Span>, // optional label for the Switch, also clickable
    styles: UiComponentStyles,
    hovered_styles: Option<UiComponentStyles>,
    checked_styles: Option<UiComponentStyles>,
    disabled_styles: Option<UiComponentStyles>,
    hover_border_size: Option<f32>,
    mouse_state: SwitchStateHandle,
    tooltip: Option<TooltipConfig>,
}

/// State handles necessary for the Switch component. Two mouse state handles are needed to handle
/// clicks on the entire component while having a hover on only the thumb.
#[derive(Default, Clone)]
pub struct SwitchStateHandle {
    component_mouse_state: MouseStateHandle,
    thumb_mouse_state: MouseStateHandle,
}

impl UiComponent for Switch {
    type ElementType = Hoverable;
    fn build(self) -> Hoverable {
        let tooltip = self.tooltip.clone();

        let hoverable = Hoverable::new(self.mouse_state.component_mouse_state.clone(), |state| {
            let styles = self.styles(state);
            let thumb_height = styles.height.unwrap_or(DEFAULT_THUMB_HEIGHT);

            let switch_element = self.render_switch(styles);
            let switch_element = if let Some(label) = self.label.clone() {
                let label = label.with_style(self.styles).build();
                let font_size = self.styles.font_size.unwrap_or_default();

                // If the thumb is larger than the label font, apply padding so the switch is
                // centered with the label.
                let padding_top = if thumb_height > font_size {
                    (thumb_height - font_size) / 2.
                } else {
                    0.
                };

                Flex::row()
                    .with_child(label.finish())
                    .with_child(
                        Container::new(switch_element)
                            .with_padding_top(padding_top)
                            .finish(),
                    )
                    .finish()
            } else {
                switch_element
            };

            // If a tooltip is configured and we're hovered, show it above the switch
            if let Some(TooltipConfig { text, styles }) = &tooltip
                && state.is_hovered()
            {
                let tooltip_element = Tooltip::new(text.clone(), *styles).build().finish();
                return Stack::new()
                    .with_child(switch_element)
                    .with_positioned_child(
                        tooltip_element,
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., -3.),
                            ParentOffsetBounds::Unbounded,
                            ParentAnchor::TopRight,
                            ChildAnchor::BottomRight,
                        ),
                    )
                    .finish();
            }

            switch_element
        });

        if !self.disabled {
            hoverable.with_cursor(Cursor::PointingHand)
        } else {
            hoverable
        }
    }

    /// Overwrites _some_ styles passed in `style` parameter
    fn with_style(self, styles: UiComponentStyles) -> Self {
        Self {
            checked: self.checked,
            disabled: self.disabled,
            label: self.label,
            styles: self.styles.merge(styles),
            hovered_styles: Some(self.hovered_styles.unwrap_or(self.styles).merge(styles)),
            checked_styles: Some(self.checked_styles.unwrap_or(self.styles).merge(styles)),
            disabled_styles: Some(self.disabled_styles.unwrap_or(self.styles).merge(styles)),
            mouse_state: self.mouse_state,
            hover_border_size: self.hover_border_size,
            tooltip: self.tooltip,
        }
    }
}

impl Switch {
    pub fn new(
        mouse_state: SwitchStateHandle,
        default_styles: UiComponentStyles,
        hovered_styles: Option<UiComponentStyles>,
        checked_styles: Option<UiComponentStyles>,
        disabled_styles: Option<UiComponentStyles>,
    ) -> Self {
        Self {
            checked: false,
            disabled: false,
            label: None,
            styles: default_styles,
            hovered_styles,
            checked_styles,
            disabled_styles,
            mouse_state,
            hover_border_size: None,
            tooltip: None,
        }
    }

    /// Sets the a circular hover border on the thumb of size `border_size`.
    pub fn with_thumb_hover_border(mut self, border_size: f32) -> Self {
        self.hover_border_size = Some(border_size);
        self
    }

    pub fn with_disabled_styles(mut self, styles: UiComponentStyles) -> Self {
        self.disabled_styles = Some(self.disabled_styles.unwrap_or_default().merge(styles));
        self
    }

    pub fn check(mut self, check: bool) -> Self {
        self.checked = check;
        self
    }

    pub fn disable(mut self) -> Self {
        self.disabled = true;
        self
    }

    pub fn with_disabled(mut self, is_disabled: bool) -> Self {
        self.disabled = is_disabled;
        self
    }

    pub fn label(mut self, label: Span) -> Self {
        self.label = Some(label);
        self
    }

    /// Adds a tooltip that appears above the switch on hover.
    pub fn with_tooltip(mut self, config: TooltipConfig) -> Self {
        self.tooltip = Some(config);
        self
    }

    fn styles(&self, state: &MouseState) -> UiComponentStyles {
        if self.disabled {
            return self.disabled_styles.unwrap_or(self.styles);
        }

        if self.checked {
            return self.checked_styles.unwrap_or(self.styles);
        }

        if state.is_mouse_over_element() {
            return self.hovered_styles.unwrap_or(self.styles);
        }
        self.styles
    }

    // Renders the thumb. The thumb needs its own hoverable to render a border around itself when
    // hovered.
    fn render_thumb(&self, styles: UiComponentStyles, thumb_height: f32) -> Box<dyn Element> {
        let is_disabled = self.disabled;
        let thumb_color = styles.foreground.unwrap_or(Fill::Solid(ColorU::white()));
        Hoverable::new(self.mouse_state.thumb_mouse_state.clone(), |state| {
            let thumb = Container::new(
                ConstrainedBox::new(
                    Rect::new()
                        .with_background(thumb_color)
                        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                        .with_drop_shadow(*DROP_SHADOW)
                        .finish(),
                )
                .with_width(thumb_height)
                .with_height(thumb_height)
                .finish(),
            )
            .finish();
            let mut stack = Stack::new();

            // If a border is specified and the mouse is over the element,
            // render a circle behind the thumb with the border color.
            if let Some(border_size) = self.hover_border_size
                && !is_disabled
                && state.is_mouse_over_element()
            {
                let mut hover_background = *TRACK_COLOR;
                hover_background.a = 100;

                let hover_size = thumb_height + border_size;

                let thumb_hover = Container::new(
                    ConstrainedBox::new(
                        Rect::new()
                            .with_background_color(hover_background)
                            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                            .finish(),
                    )
                    .with_width(hover_size)
                    .with_height(hover_size)
                    .finish(),
                )
                .finish();

                // Position the hover so that it's centered around the thumb. Since the hover
                // is guaranteed to be larger than the thumb, we position the hover at the top
                // left corner of the thumb and then translate it to the left and up so that it
                // is centered.
                stack.add_positioned_child(
                    thumb_hover,
                    OffsetPositioning::from_axes(
                        PositioningAxis::relative_to_parent(
                            ParentOffsetBounds::Unbounded,
                            OffsetType::Pixel(-((hover_size - thumb_height) / 2.)),
                            AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                        ),
                        PositioningAxis::relative_to_parent(
                            ParentOffsetBounds::Unbounded,
                            OffsetType::Pixel(-((hover_size - thumb_height) / 2.)),
                            AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                        ),
                    ),
                );
            }

            stack.add_child(thumb);
            stack.finish()
        })
        .finish()
    }

    fn render_switch(&self, styles: UiComponentStyles) -> Box<dyn Element> {
        let thumb_height = styles.height.unwrap_or(DEFAULT_THUMB_HEIGHT);

        let track = Container::new(
            ConstrainedBox::new(Empty::new().finish())
                .with_width(thumb_height * 2.)
                .with_height(thumb_height)
                .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)));

        let background_color = styles.background.unwrap_or(Fill::Solid(*TRACK_COLOR));

        let mut stack = Stack::new();
        stack.add_child(track.with_background(background_color).finish());

        let thumb = self.render_thumb(styles, thumb_height);

        // If checked, render the thumb's right corner on the right corner of the track. If
        // unchecked, render the thumb's left corner on the left corner of the track.
        let positioning = if self.checked {
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_parent(
                    ParentOffsetBounds::Unbounded,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Right),
                ),
                PositioningAxis::relative_to_parent(
                    ParentOffsetBounds::Unbounded,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                ),
            )
        } else {
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            )
        };

        stack.add_positioned_child(thumb, positioning);
        stack.finish()
    }
}
