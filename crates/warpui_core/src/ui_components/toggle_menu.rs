use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

use lazy_static::lazy_static;
use parking_lot::Mutex;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;

use super::components::{UiComponent, UiComponentStyles};
use super::text::Span;
use crate::elements::{
    Container, CrossAxisAlignment, Empty, Flex, Hoverable, MainAxisSize, MouseStateHandle,
    ParentElement, Shrinkable,
};
use crate::platform::Cursor;
use crate::scene::{CornerRadius, Radius};
use crate::{AppContext, Element, EventContext};

const BORDER_RADIUS: f32 = 4.;
const BUTTON_VERTICAL_PADDING: f32 = 2.;
const BUTTON_MARGIN: f32 = 4.;

lazy_static! {
    pub static ref FALLBACK_SELECTED_COLOR: ColorU = ColorU::new(64, 64, 64, 100);
    pub static ref FALLBACK_BACKGROUND_COLOR: ColorU = ColorU::new(25, 25, 25, 100);
}

pub struct ToggleMenuItem {
    label: Cow<'static, str>,
}

impl ToggleMenuItem {
    pub fn new(label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ToggleMenuState {
    selected_item: Option<usize>,
    default_selected_item: Option<usize>,
}

#[derive(Clone, Default)]
pub struct ToggleMenuStateHandle {
    inner: Arc<Mutex<ToggleMenuState>>,
}

impl ToggleMenuStateHandle {
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

    fn set(&self, new_state: ToggleMenuState) {
        let mut guard = self.inner.lock();
        *guard = new_state;
    }

    // Set the active index from outside of the toggle menu component
    pub fn set_selected_idx(&self, new_idx: usize) {
        let default_selected_item = self.get_default_idx();
        self.set(ToggleMenuState {
            selected_item: Some(new_idx),
            default_selected_item,
        });
    }
}

struct ToggleMenuRenderer {
    default_styles: UiComponentStyles,
    selected_styles: UiComponentStyles,
    hovered_styles: UiComponentStyles,
    state_handle: ToggleMenuStateHandle,
    hover_states: Vec<MouseStateHandle>,
    is_disabled: bool,
}

impl ToggleMenuRenderer {
    fn render_label(&self, label: Cow<'static, str>) -> Box<dyn Element> {
        let font_styles = UiComponentStyles {
            font_family_id: self.default_styles.font_family_id,
            font_size: self.default_styles.font_size,
            font_color: self
                .default_styles
                .font_color
                .unwrap_or_else(ColorU::white)
                .into(),
            font_weight: self.default_styles.font_weight,
            ..Default::default()
        };

        Span::new(label, font_styles)
            .with_soft_wrap()
            .build()
            .finish()
    }

    fn render_item(
        &self,
        item_idx: usize,
        item: ToggleMenuItem,
        on_toggle_change: Rc<ToggleMenuCallback>,
    ) -> Box<dyn Element> {
        let selected = self
            .state_handle
            .get_selected_idx()
            .map(|selected_idx| selected_idx == item_idx)
            .unwrap_or(false);

        let mut hoverable = Hoverable::new(self.hover_states[item_idx].clone(), |state| {
            let ToggleMenuItem { label } = item;

            let flex_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1., Empty::new().finish()).finish())
                .with_child(self.render_label(label))
                .with_child(Shrinkable::new(1., Empty::new().finish()).finish())
                .finish();

            let mut container = Container::new(flex_row);

            if let Some(padding) = self.default_styles.padding {
                container = container
                    .with_padding_bottom(padding.bottom)
                    .with_padding_left(padding.left)
                    .with_padding_right(padding.right)
                    .with_padding_top(padding.top);
            } else {
                container = container.with_vertical_padding(BUTTON_VERTICAL_PADDING)
            }

            if let Some(margin) = self.default_styles.margin {
                container = container
                    .with_margin_bottom(margin.bottom)
                    .with_margin_left(margin.left)
                    .with_margin_right(margin.right)
                    .with_margin_top(margin.top);
            } else if item_idx == 0 {
                container = container.with_uniform_margin(BUTTON_MARGIN);
            } else {
                container = container
                    .with_margin_right(BUTTON_MARGIN)
                    .with_vertical_margin(BUTTON_MARGIN);
            }

            if let Some(radius) = self.default_styles.border_radius {
                container = container.with_corner_radius(radius);
            } else {
                container = container
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(BORDER_RADIUS)));
            }

            if selected {
                container = container.with_background(
                    self.selected_styles
                        .background
                        .unwrap_or((*FALLBACK_SELECTED_COLOR).into()),
                );
            } else if !self.is_disabled && state.is_hovered() {
                container = container.with_background(
                    self.hovered_styles
                        .background
                        .unwrap_or((*FALLBACK_SELECTED_COLOR).into()),
                );
            }

            container.finish()
        });

        let state_handle = self.state_handle.clone();
        let old_default = state_handle.get_default_idx();
        if !self.is_disabled {
            hoverable = hoverable
                .on_click(move |event_ctx, app, v2f| {
                    // Trigger the callback if a new item is selected
                    if state_handle.get_selected_idx() != Some(item_idx) {
                        on_toggle_change(event_ctx, app, v2f);

                        state_handle.set(ToggleMenuState {
                            selected_item: Some(item_idx),
                            default_selected_item: old_default,
                        });
                    }
                })
                .with_cursor(Cursor::PointingHand);
        }

        hoverable.finish()
    }
}

pub type ToggleMenuCallback = dyn Fn(&mut EventContext, &AppContext, Vector2F) + 'static;

pub struct ToggleMenu {
    items: Vec<ToggleMenuItem>,
    renderer: ToggleMenuRenderer,
    /// Callback function to be run when the toggle state is changed.
    on_toggle_change: Rc<ToggleMenuCallback>,
}

impl UiComponent for ToggleMenu {
    type ElementType = Container;

    fn build(self) -> Self::ElementType {
        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children(self.items.into_iter().enumerate().map(|(idx, item)| {
                    Shrinkable::new(
                        1.,
                        Container::new(self.renderer.render_item(
                            idx,
                            item,
                            self.on_toggle_change.clone(),
                        ))
                        .finish(),
                    )
                    .finish()
                }))
                .with_main_axis_size(MainAxisSize::Max)
                .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_background(
            self.renderer
                .default_styles
                .background
                .unwrap_or((*FALLBACK_BACKGROUND_COLOR).into()),
        )
    }

    fn with_style(self, new_styles: UiComponentStyles) -> Self {
        Self {
            renderer: ToggleMenuRenderer {
                default_styles: new_styles.merge(self.renderer.default_styles),
                ..self.renderer
            },
            ..self
        }
    }
}

impl ToggleMenu {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mouse_states: Vec<MouseStateHandle>,
        items: Vec<ToggleMenuItem>,
        toggle_menu_state_handle: ToggleMenuStateHandle,
        default_option: Option<usize>,
        default_styles: UiComponentStyles,
        selected_styles: UiComponentStyles,
        hovered_styles: UiComponentStyles,
        on_toggle_change: Rc<ToggleMenuCallback>,
    ) -> Self {
        let mut selected_idx = toggle_menu_state_handle.get_selected_idx();
        if let Some(id) = selected_idx
            && items.get(id).is_none()
        {
            // Previously selected option is out of range, reset to default.
            selected_idx = None
        }

        toggle_menu_state_handle.set(ToggleMenuState {
            selected_item: selected_idx,
            default_selected_item: default_option,
        });
        Self {
            items,
            renderer: ToggleMenuRenderer {
                default_styles,
                selected_styles,
                hovered_styles,
                state_handle: toggle_menu_state_handle,
                hover_states: mouse_states,
                is_disabled: false,
            },
            on_toggle_change,
        }
    }

    pub fn with_disabled(mut self, is_disabled: bool) -> Self {
        self.renderer.is_disabled = is_disabled;
        self
    }
}
