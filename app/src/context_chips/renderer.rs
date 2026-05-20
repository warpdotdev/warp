//! The renderer for a single context chip.

use pathfinder_color::ColorU;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    ConstrainedBox, DraggableState, Hoverable, MouseStateHandle, OffsetPositioning, ParentElement,
    ParentOffsetBounds, Stack,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::Action;
use warpui::{
    elements::{Container, CrossAxisAlignment, Flex, Text},
    Element,
};

use crate::appearance::Appearance;
use crate::ui_components::icons;

use super::context_chip::ContextChip;
use super::display_chip::{chip_container, udi_font_size};
use super::spacing;
use super::{ChipAvailability, ChipValue, ContextChipKind};
use pathfinder_geometry::vector::vec2f;

/// Styling consts.
const CORNER_RADIUS_PIXELS: f32 = 4.;
const ICON_MARGIN_RIGHT: f32 = 6.;
const LABEL_MARGIN_BOTTOM: f32 = 6.;

#[derive(Clone)]
pub struct RendererStyles {
    pub value_color: ColorU,
    pub font_properties: Properties,
}

impl RendererStyles {
    pub fn new(value_color: ColorU, font_properties: Properties) -> Self {
        Self {
            value_color,
            font_properties,
        }
    }
}

#[derive(Clone, Copy)]
pub enum ChipDragState {
    Draggable { is_dragging: bool },
    Undraggable,
}

#[derive(Clone)]
/// State for rendering a single context chip.
pub struct Renderer {
    kind: ContextChipKind,
    chip: ContextChip,
    value: ChipValue,
    styles: RendererStyles,
    draggable_state: DraggableState,
    tooltip_state_handle: MouseStateHandle,
    remove_button_state_handle: MouseStateHandle,
    is_disabled: bool,
    tooltip_override_text: Option<String>,
}

impl Renderer {
    pub fn new(
        kind: ContextChipKind,
        chip: ContextChip,
        value: ChipValue,
        styles: RendererStyles,
        availability: ChipAvailability,
    ) -> Self {
        let is_disabled = !availability.is_enabled();
        let tooltip_override_text = availability.tooltip_override_text();
        Self {
            kind,
            chip,
            value,
            styles,
            draggable_state: Default::default(),
            tooltip_state_handle: Default::default(),
            remove_button_state_handle: Default::default(),
            is_disabled,
            tooltip_override_text,
        }
    }

    pub fn default_from_kind(
        chip_kind: ContextChipKind,
        availability: ChipAvailability,
        appearance: &Appearance,
    ) -> Option<Self> {
        Self::default_from_kind_with_agent_view(chip_kind, availability, false, appearance)
    }

    pub fn default_from_kind_with_agent_view(
        chip_kind: ContextChipKind,
        availability: ChipAvailability,
        is_in_agent_view: bool,
        appearance: &Appearance,
    ) -> Option<Self> {
        let chip = chip_kind.to_chip()?;
        let placeholder_value = chip_kind.placeholder_value();
        let styles = chip_kind.default_styles(appearance, is_in_agent_view);
        Some(Self::new(
            chip_kind,
            chip,
            placeholder_value,
            styles,
            availability,
        ))
    }

    pub fn draggable_state(&self) -> DraggableState {
        self.draggable_state.clone()
    }

    pub fn chip_kind(&self) -> &ContextChipKind {
        &self.kind
    }

    pub fn is_disabled(&self) -> bool {
        self.is_disabled
    }

    fn render_remove_button<A: Action + Clone>(
        &self,
        action: A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_size = appearance.monospace_font_size();
        let button = Hoverable::new(self.remove_button_state_handle.clone(), |_| {
            ConstrainedBox::new(
                icons::Icon::X
                    .to_warpui_icon(appearance.theme().ui_error_color().into())
                    .finish(),
            )
            .with_height(icon_size)
            .with_width(icon_size)
            .finish()
        });
        button
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .with_cursor(Cursor::PointingHand)
            .finish()
    }

    fn render_internal(
        &self,
        drag_state: ChipDragState,
        remove_button: Option<Box<dyn Element>>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut color = self.styles.value_color;
        if self.is_disabled {
            color.a = (color.a / 2).max(48);
        }
        let font_size = udi_font_size(appearance);

        let mut content = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(icon) = self.kind.udi_icon() {
            content.add_child(
                Container::new(
                    ConstrainedBox::new(icon.to_warpui_icon(Fill::Solid(color)).finish())
                        .with_height(font_size)
                        .with_width(font_size)
                        .finish(),
                )
                .with_margin_right(spacing::UDI_CHIP_ICON_GAP)
                .finish(),
            );
        }

        let text = Text::new_inline(
            self.value.to_string(),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(color)
        .with_line_height_ratio(appearance.line_height_ratio())
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish();
        content.add_child(text);

        if let Some(remove_button) = remove_button {
            content.add_child(
                Container::new(remove_button)
                    .with_margin_left(spacing::UDI_CHIP_ICON_GAP)
                    .finish(),
            );
        }

        let container = chip_container(content.finish(), None, appearance);

        let mut hoverable = Hoverable::new(self.tooltip_state_handle.clone(), |mouse_state| {
            if !mouse_state.is_hovered()
                || matches!(drag_state, ChipDragState::Draggable { is_dragging: true })
            {
                return container.finish();
            }

            let tooltip = appearance.ui_builder().tool_tip(
                self.tooltip_override_text
                    .clone()
                    .unwrap_or_else(|| self.chip.title().to_string()),
            );
            let mut stack = Stack::new();
            stack.add_child(container.finish());
            stack.add_positioned_overlay_child(
                tooltip.build().finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -2.5 * font_size),
                    ParentOffsetBounds::Unbounded,
                    warpui::elements::ParentAnchor::Center,
                    warpui::elements::ChildAnchor::Center,
                ),
            );
            stack.finish()
        });

        if matches!(drag_state, ChipDragState::Draggable { .. }) && !self.is_disabled {
            hoverable = hoverable.with_cursor(Cursor::OpenHand);
        }

        hoverable.finish()
    }

    pub fn render_unused(
        &self,
        drag_state: ChipDragState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        self.render_internal(drag_state, None, appearance)
    }

    pub fn render_used<A: Action + Clone>(
        &self,
        drag_state: ChipDragState,
        on_remove_action: A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let remove_button =
            (!self.is_disabled).then(|| self.render_remove_button(on_remove_action, appearance));
        self.render_internal(drag_state, remove_button, appearance)
    }
}

#[cfg(test)]
#[path = "renderer_tests.rs"]
mod tests;
