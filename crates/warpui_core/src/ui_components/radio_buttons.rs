use std::borrow::Cow;
use std::rc::Rc;

use lazy_static::lazy_static;
use parking_lot::Mutex;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;

use super::components::{Coords, UiComponent, UiComponentStyles};
use crate::elements::{
    ChildAnchor, ConstrainedBox, Container, CrossAxisAlignment, Flex, FormattedTextElement,
    Hoverable, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Rect, Stack,
};
use crate::platform::Cursor;
use crate::scene::{Border, CornerRadius, Radius};
use crate::{AppContext, Element, EventContext};

const LABEL_LEFT_MARGIN: f32 = 8.;
const BORDER_WIDTH: f32 = 1.5;
const DEFAULT_FONT_SIZE: f32 = 14.;
const HOVER_SIZE_MULTIPLE: f32 = 1.75;
const RADIO_BUTTON_DIAMETER: f32 = 20.;

lazy_static! {
    pub static ref HOVER_BACKGROUND_COLOR: ColorU = ColorU::new(170, 170, 170, 50);
}

pub enum RadioButtonLayout {
    Row,
    Column,
}

/// A function from (is_disabled, is_selected, hovered) to a rendered element.
type RichLabelFn<'a> = dyn FnOnce(bool, bool, bool) -> Box<dyn Element> + 'a;
/// A function from (is_disabled, is_selected, hovered) to a rendered element.
type CustomItemFn<'a> = dyn FnOnce(bool, bool, bool) -> Box<dyn Element> + 'a;

pub enum Label<'a> {
    Text(Cow<'static, str>),
    Rich(Box<RichLabelFn<'a>>),
    CustomItem(Box<CustomItemFn<'a>>),
}

pub struct RadioButtonItem<'a> {
    is_disabled: bool,
    child: Label<'a>,
}

impl<'a> RadioButtonItem<'a> {
    fn new(child: Label<'a>) -> Self {
        Self {
            is_disabled: false,
            child,
        }
    }

    pub fn text(label: impl Into<Cow<'static, str>>) -> Self {
        Self::new(Label::Text(label.into()))
    }

    pub fn rich_element(label: Box<RichLabelFn<'a>>) -> Self {
        Self::new(Label::Rich(Box::new(label)))
    }

    pub fn custom_item(label: Box<CustomItemFn<'a>>) -> Self {
        Self::new(Label::CustomItem(Box::new(label)))
    }

    pub fn with_disabled(mut self, is_disabled: bool) -> Self {
        self.is_disabled = is_disabled;
        self
    }
}

#[derive(Clone, Copy, Default)]
struct RadioButtonState {
    selected_item: Option<usize>,
    default_selected_item: Option<usize>,
}

impl RadioButtonState {
    #[allow(dead_code)] // This is a temporary constructor that isn't used right now but will be used as soon as radio buttons are used.
    pub fn new(default_selected_item: Option<usize>) -> Self {
        RadioButtonState {
            selected_item: None,
            default_selected_item,
        }
    }
}

#[derive(Clone, Default)]
pub struct RadioButtonStateHandle {
    inner: Rc<Mutex<RadioButtonState>>,
}

// TODO(roland): Remembering the selected option can be unintuitive if the number of options
// changes or options become disabled/enabled. The remembered index can be semantically different
// if the number/content of options change, and we may not want to remember an option chosen only
// because other options were disabled. Consider a refactor.
impl RadioButtonStateHandle {
    pub fn get_selected_idx(&self) -> Option<usize> {
        let state = self.inner.lock();
        match (state.selected_item, state.default_selected_item) {
            (Some(selected_idx), _) => Some(selected_idx),
            (None, Some(default_idx)) => Some(default_idx),
            _ => None,
        }
    }

    fn get_default_idx(&self) -> Option<usize> {
        let state = self.inner.lock();
        state.default_selected_item
    }

    fn set(&self, new_state: RadioButtonState) {
        let mut guard = self.inner.lock();
        *guard = new_state;
    }

    // Set the active index from outside of the radio button component
    pub fn set_selected_idx(&self, new_idx: usize) {
        let default_selected_item = self.get_default_idx();
        self.set(RadioButtonState {
            selected_item: Some(new_idx),
            default_selected_item,
        });
    }
}

struct RadioButtonRenderer {
    layout: RadioButtonLayout,
    default_styles: UiComponentStyles,
    selected_styles: UiComponentStyles,
    disabled_styles: UiComponentStyles,
    state_handle: RadioButtonStateHandle,
    hover_states: Vec<MouseStateHandle>,
    /// If None, then center the button relative to its child.
    /// Otherwise, insert a margin on the top edge.
    button_vertical_offset: Option<f32>,
    change_handler: Option<Rc<OnChangeFn>>,
    supports_unselected_state: bool,
    button_diameter_override: Option<f32>,
}

impl RadioButtonRenderer {
    fn render_selection_circle(&self, selected: bool, is_disabled: bool) -> Box<dyn Element> {
        let mut stack = Stack::new();
        let diameter = self
            .button_diameter_override
            .unwrap_or(RADIO_BUTTON_DIAMETER);

        let mut outer_circle_rect =
            Rect::new().with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)));
        if let Some(background) = self.default_styles.background {
            outer_circle_rect = outer_circle_rect.with_background(background);
        }
        let border_color = if selected {
            self.selected_styles.border_color.unwrap_or_default()
        } else if is_disabled {
            self.disabled_styles.border_color.unwrap_or_default()
        } else {
            self.default_styles.border_color.unwrap_or_default()
        };
        outer_circle_rect =
            outer_circle_rect.with_border(Border::all(BORDER_WIDTH).with_border_fill(border_color));
        let outer_circle = ConstrainedBox::new(outer_circle_rect.finish())
            .with_height(diameter)
            .with_width(diameter);

        stack.add_child(outer_circle.finish());

        if selected {
            let inner_circle_rect = Rect::new()
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .with_background(self.selected_styles.background.unwrap_or_default());
            let inner_circle_diameter = diameter / 2.;
            let inner_circle = ConstrainedBox::new(inner_circle_rect.finish())
                .with_height(inner_circle_diameter)
                .with_width(inner_circle_diameter);

            // Position the inner circle so that it's centered in the outer circle.
            stack.add_positioned_child(
                inner_circle.finish(),
                OffsetPositioning::offset_from_parent(
                    Vector2F::zero(),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
        }

        stack.finish()
    }

    fn render_label(&self, label: Cow<'static, str>, is_disabled: bool) -> Box<dyn Element> {
        let color = if is_disabled {
            self.disabled_styles.font_color
        } else {
            self.default_styles.font_color
        }
        .unwrap_or_else(ColorU::white);

        FormattedTextElement::from_str(
            label,
            self.default_styles.font_family_id.expect("No font family"),
            self.default_styles.font_size.unwrap_or(DEFAULT_FONT_SIZE),
        )
        .with_color(color)
        .with_weight(self.default_styles.font_weight.unwrap_or_default())
        .finish()
    }

    fn render_item(&self, item_idx: usize, item: RadioButtonItem) -> Box<dyn Element> {
        let selected = self
            .state_handle
            .get_selected_idx()
            .map(|selected_idx| selected_idx == item_idx)
            .unwrap_or(false);

        let padding = self.default_styles.padding.unwrap_or(Coords::uniform(2.));

        let (left_padding, top_padding) = match (item_idx, &self.layout) {
            (0, RadioButtonLayout::Column) => (padding.left, 0.),
            (0, RadioButtonLayout::Row) => (0., padding.top),
            _ => (padding.left, padding.top),
        };

        let mut hoverable = Hoverable::new(self.hover_states[item_idx].clone(), |state| {
            if let Label::CustomItem(build_child) = item.child {
                return (build_child)(item.is_disabled, selected, state.is_hovered());
            }
            let mut stack = Stack::new();

            let button = self.render_selection_circle(selected, item.is_disabled);

            let circle_diameter = self.default_styles.font_size.unwrap_or_default();
            let hover_size = circle_diameter * HOVER_SIZE_MULTIPLE;
            if !item.is_disabled && state.is_hovered() {
                let hover = Container::new(
                    ConstrainedBox::new(
                        Rect::new()
                            .with_background_color(*HOVER_BACKGROUND_COLOR)
                            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                            .finish(),
                    )
                    .with_width(hover_size)
                    .with_height(hover_size)
                    .finish(),
                )
                .finish();

                // Position the hover so that it's centered behind the circle.
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

            stack.add_child(button);

            let container = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(if let Some(offset) = self.button_vertical_offset {
                        Container::new(stack.finish())
                            .with_margin_top(offset)
                            .finish()
                    } else {
                        stack.finish()
                    })
                    .with_child(match item.child {
                        Label::Text(label) => {
                            Container::new(self.render_label(label, item.is_disabled))
                                .with_margin_left(LABEL_LEFT_MARGIN)
                                .finish()
                        }
                        Label::Rich(build_child) => {
                            (build_child)(item.is_disabled, selected, state.is_hovered())
                        }
                        _ => Flex::row().finish(),
                    })
                    .with_cross_axis_alignment(if self.button_vertical_offset.is_some() {
                        CrossAxisAlignment::Start
                    } else {
                        CrossAxisAlignment::Center
                    })
                    .finish(),
            );
            container
                .with_padding_top(top_padding)
                .with_padding_bottom(padding.bottom)
                .with_padding_left(left_padding)
                .with_padding_right(padding.right)
                .finish()
        });

        if !item.is_disabled {
            let state_handle = self.state_handle.clone();
            let old_default = state_handle.get_default_idx();
            let change_handler = self.change_handler.clone();
            let supports_unselected = self.supports_unselected_state;
            hoverable = hoverable
                .on_click(move |event_context, app_context, _| {
                    let selected_item = if supports_unselected && selected {
                        None
                    } else {
                        Some(item_idx)
                    };
                    state_handle.set(RadioButtonState {
                        selected_item,
                        default_selected_item: old_default,
                    });
                    if let Some(change_handler) = &change_handler {
                        change_handler(event_context, app_context, selected_item);
                    }
                })
                .with_cursor(Cursor::PointingHand);
        }

        let margin = self.default_styles.margin.unwrap_or(Coords::uniform(2.));

        let (left_margin, top_margin) = match (item_idx, &self.layout) {
            (0, RadioButtonLayout::Column) => (margin.left, 0.),
            (0, RadioButtonLayout::Row) => (0., margin.top),
            _ => (margin.left, margin.top),
        };

        let container = Container::new(hoverable.finish());
        container
            .with_margin_top(top_margin)
            .with_margin_bottom(margin.bottom)
            .with_margin_left(left_margin)
            .with_margin_right(margin.right)
            .finish()
    }
}

type OnChangeFn = dyn Fn(&mut EventContext, &AppContext, Option<usize>) + 'static;

pub struct RadioButtons<'a> {
    items: Vec<RadioButtonItem<'a>>,
    renderer: RadioButtonRenderer,
}

impl UiComponent for RadioButtons<'_> {
    type ElementType = Flex;

    fn build(self) -> Self::ElementType {
        let flex = match self.renderer.layout {
            RadioButtonLayout::Row => Flex::row(),
            RadioButtonLayout::Column => Flex::column(),
        };

        flex.with_children(
            self.items
                .into_iter()
                .enumerate()
                .map(|(idx, item)| self.renderer.render_item(idx, item)),
        )
    }

    fn with_style(self, new_styles: UiComponentStyles) -> Self {
        Self {
            renderer: RadioButtonRenderer {
                default_styles: self.renderer.default_styles.merge(new_styles),
                ..self.renderer
            },
            ..self
        }
    }
}

impl<'a> RadioButtons<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mouse_states: Vec<MouseStateHandle>,
        items: Vec<RadioButtonItem<'a>>,
        radio_button_state_handle: RadioButtonStateHandle,
        default_option: Option<usize>,
        default_styles: UiComponentStyles,
        selected_styles: UiComponentStyles,
        disabled_styles: UiComponentStyles,
        layout: RadioButtonLayout,
    ) -> Self {
        let mut selected_idx = radio_button_state_handle.get_selected_idx();
        if let Some(id) = selected_idx {
            // If the previously selected option is disabled, reset the selected option to the default.
            if let Some(item) = items.get(id) {
                if item.is_disabled {
                    selected_idx = None
                }
            } else {
                // Previously selected option is out of range, reset to default.
                selected_idx = None
            }
        }

        radio_button_state_handle.set(RadioButtonState {
            selected_item: selected_idx,
            default_selected_item: default_option,
        });
        Self {
            items,
            renderer: RadioButtonRenderer {
                layout,
                default_styles,
                selected_styles,
                disabled_styles,
                state_handle: radio_button_state_handle,
                hover_states: mouse_states,
                button_vertical_offset: None,
                change_handler: None,
                supports_unselected_state: false,
                button_diameter_override: None,
            },
        }
    }

    /// Set the vertical offset of the radio button relative to the top of the child element.
    pub fn with_button_vertical_offset(mut self, offset: f32) -> Self {
        self.renderer.button_vertical_offset = Some(offset);
        self
    }

    pub fn on_change(mut self, callback: Rc<OnChangeFn>) -> Self {
        self.renderer.change_handler = Some(callback);
        self
    }

    pub fn supports_unselected_state(mut self) -> Self {
        self.renderer.supports_unselected_state = true;
        self
    }

    pub fn with_button_diameter(mut self, diameter: f32) -> Self {
        self.renderer.button_diameter_override = Some(diameter);
        self
    }
}
