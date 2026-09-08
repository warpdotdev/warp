use lazy_static::lazy_static;

use crate::color::ColorU;
use crate::elements::{
    Align, Border, ChildAnchor, ConstrainedBox, Container, Element, Flex, Hoverable, Icon,
    MouseState, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Rect, Stack,
};
use crate::geometry::vector::Vector2F;
use crate::prelude::{Coords, Fill};
use crate::ui_components::components::{UiComponent, UiComponentStyles};
use crate::ui_components::text::Span;

const CHECK_SVG_PATH: &str = "bundled/svg/check-thick.svg";

/// Number of pixels that should be between the checkmark and checkbox.
const CHECKMARK_LENGTH_ADJUSTMENT: f32 = 3.;

const LABEL_LEFT_MARGIN: f32 = 4.;

lazy_static! {
    pub static ref HOVER_BACKGROUND_COLOR: ColorU = ColorU::new(170, 170, 170, 50);
}

pub struct Checkbox {
    default_styles: UiComponentStyles,
    hovered_styles: Option<UiComponentStyles>,
    checked_styles: Option<UiComponentStyles>,
    disabled_styles: Option<UiComponentStyles>,
    hover_state: MouseStateHandle,

    disabled: bool,
    checked: bool,

    /// Optional, clickable text rendered to the right of the checkbox
    label: Option<Span>,
}

impl UiComponent for Checkbox {
    type ElementType = Hoverable;
    fn build(self) -> Hoverable {
        let hoverable = Hoverable::new(self.hover_state.clone(), |state| {
            let checkbox = self.render_checkbox(state);
            if let Some(label) = self.label.clone() {
                Flex::row()
                    .with_cross_axis_alignment(crate::elements::CrossAxisAlignment::Center)
                    .with_child(checkbox)
                    .with_child(
                        Container::new(label.with_style(self.styles(state)).build().finish())
                            .with_margin_left(LABEL_LEFT_MARGIN)
                            .finish(),
                    )
                    .finish()
            } else {
                checkbox
            }
        });
        if self.disabled {
            return hoverable.disable();
        }
        hoverable
    }

    /// Overwrites _some_ styles passed in `style` parameter
    fn with_style(self, styles: UiComponentStyles) -> Self {
        Self {
            default_styles: self.default_styles.merge(styles),
            hovered_styles: Some(
                self.hovered_styles
                    .unwrap_or(self.default_styles)
                    .merge(styles),
            ),
            checked_styles: Some(
                self.checked_styles
                    .unwrap_or(self.default_styles)
                    .merge(styles),
            ),
            disabled_styles: Some(
                self.disabled_styles
                    .unwrap_or(self.default_styles)
                    .merge(styles),
            ),
            ..self
        }
    }
}

impl Checkbox {
    pub fn new(
        mouse_state: MouseStateHandle,
        default_styles: UiComponentStyles,
        hovered_styles: Option<UiComponentStyles>,
        checked_styles: Option<UiComponentStyles>,
        disabled_styles: Option<UiComponentStyles>,
    ) -> Self {
        Self {
            default_styles,
            hovered_styles,
            checked_styles,
            disabled_styles,
            hover_state: mouse_state,
            disabled: false,
            checked: false,
            label: None,
        }
    }

    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    pub fn check(mut self, check: bool) -> Self {
        self.checked = check;
        self
    }

    pub fn with_label(mut self, label: Span) -> Self {
        self.label = Some(label);
        self
    }

    fn styles(&self, state: &MouseState) -> UiComponentStyles {
        let styles = if self.disabled {
            self.disabled_styles
        } else if self.checked || state.is_clicked() {
            self.checked_styles
        } else {
            None
        };
        styles.unwrap_or(self.default_styles)
    }

    // If checked, use the icon with the appropriate color. Otherwise, use an empty box.
    fn render_checkmark(&self, checked: bool, icon_color: ColorU) -> Box<dyn Element> {
        if checked {
            Icon::new(CHECK_SVG_PATH, icon_color).finish()
        } else {
            Rect::new().finish()
        }
    }

    fn render_checkbox(&self, state: &MouseState) -> Box<dyn Element> {
        let styles = self.styles(state);

        let border_width = styles.border_width;

        // The full length of the checkbox will be the font size, but we need
        // to account for the border on each side of the length.
        let checkbox_length = styles.font_size.unwrap_or_default();
        let checkbox_length_without_border = checkbox_length - 2. * border_width.unwrap_or(0.);

        // Use font_color for the checkmark when checked, otherwise use default foreground
        let icon_color = styles.font_color.unwrap_or_default();
        let checkmark = self.render_checkmark(self.checked, icon_color);
        let checkmark_length = checkbox_length_without_border - CHECKMARK_LENGTH_ADJUSTMENT;

        let mut checkbox = Container::new(
            ConstrainedBox::new(
                Align::new(
                    ConstrainedBox::new(checkmark)
                        .with_height(checkmark_length)
                        .with_width(checkmark_length)
                        .finish(),
                )
                .finish(),
            )
            .with_width(checkbox_length_without_border)
            .with_height(checkbox_length_without_border)
            .finish(),
        )
        .with_corner_radius(styles.border_radius.unwrap_or_default());

        if !state.is_mouse_over_element()
            && let Some(background) = styles.background
        {
            checkbox = checkbox.with_background(background);
        }

        if let Some(border_width) = border_width {
            checkbox = checkbox.with_border(
                Border::all(border_width).with_border_fill(styles.border_color.unwrap_or_default()),
            );
        }

        let mut stack = Stack::new();

        if state.is_mouse_over_element() {
            let hover = Container::new(
                ConstrainedBox::new(
                    Rect::new()
                        .with_background(
                            self.hovered_styles
                                .and_then(|styles| styles.background)
                                .unwrap_or(Fill::Solid(*HOVER_BACKGROUND_COLOR)),
                        )
                        .with_corner_radius(styles.border_radius.unwrap_or_default())
                        .finish(),
                )
                .with_width(checkbox_length)
                .with_height(checkbox_length)
                .finish(),
            )
            .finish();

            // Position the hover so that it's centered behind the checkbox.
            stack.add_positioned_child(
                hover,
                OffsetPositioning::offset_from_parent(
                    Vector2F::zero(),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
        }

        // Add the checkbox itself
        stack.add_child(checkbox.finish());

        let margin = styles
            .margin
            .unwrap_or(Coords::uniform(checkbox_length / 2.));

        Container::new(stack.finish())
            .with_margin_left(margin.left)
            .with_margin_right(margin.right)
            .with_margin_top(margin.top)
            .with_margin_bottom(margin.bottom)
            .finish()
    }
}
