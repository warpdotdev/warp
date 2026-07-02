//! Generalized drag/drop chip configurator.
//!
//! This module extracts the reusable chip arrangement logic from the prompt
//! `EditorModal` so that it can be shared between the terminal prompt editor
//! and the agent input footer editor.
pub(crate) mod modal_shell;

pub(crate) use modal_shell::{
    render_chip_editor_modal, render_chip_editor_sections, ChipEditorModalConfig,
    ChipEditorMouseHandles, ChipEditorSectionsConfig,
};
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ConstrainedBox, Container, CrossAxisAlignment, Dash, DispatchEventResult, Draggable,
    DraggableState, Element, Empty, EventHandler, Flex, Hoverable, MouseStateHandle,
    OffsetPositioning, ParentElement, ParentOffsetBounds, SavePosition, Stack, Text, Wrap,
};
use warpui::fonts::Properties;
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::{Action, View, ViewContext};

use crate::ai::blocklist::agent_view::toolbar_item::AgentToolbarItemKind;
use crate::appearance::Appearance;
use crate::context_chips::display_chip::{chip_container, udi_font_size};
use crate::context_chips::renderer::{ChipDragState, Renderer as ContextChipRenderer};
use crate::context_chips::{spacing, ChipAvailability, ContextChipKind};
use crate::ui_components::icons;

const USED_CHIPS_POSITION_ID: &str = "chip_cfg_used";
const LEFT_CHIPS_POSITION_ID: &str = "chip_cfg_left";
const RIGHT_CHIPS_POSITION_ID: &str = "chip_cfg_right";
const UNUSED_CHIPS_POSITION_ID: &str = "chip_cfg_unused";

/// An item that can be placed in the configurator — either a context chip or
/// an interactive control button.
pub enum ConfigurableItem {
    ContextChip(Box<ContextChipRenderer>),
    Control(ControlItemRenderer),
}

impl ConfigurableItem {
    pub fn from_toolbar_item(kind: AgentToolbarItemKind, appearance: &Appearance) -> Option<Self> {
        match kind {
            AgentToolbarItemKind::ContextChip(chip_kind) => {
                ContextChipRenderer::default_from_kind_with_agent_view(
                    chip_kind,
                    ChipAvailability::Enabled,
                    true,
                    appearance,
                )
                .map(Box::new)
                .map(Self::ContextChip)
            }
            control => Some(Self::Control(ControlItemRenderer::new(control))),
        }
    }

    pub fn item_kind(&self) -> Option<AgentToolbarItemKind> {
        match self {
            Self::ContextChip(r) => Some(AgentToolbarItemKind::ContextChip(r.chip_kind().clone())),
            Self::Control(r) => r.kind.clone(),
        }
    }

    pub fn chip_kind(&self) -> Option<&ContextChipKind> {
        match self {
            Self::ContextChip(r) => Some(r.chip_kind()),
            Self::Control(_) => None,
        }
    }

    pub fn is_removable(&self) -> bool {
        match self {
            Self::ContextChip(_) => true,
            Self::Control(r) => r.removable,
        }
    }

    pub fn draggable_state(&self) -> DraggableState {
        match self {
            Self::ContextChip(r) => r.draggable_state(),
            Self::Control(r) => r.draggable_state.clone(),
        }
    }

    pub fn render_unused(
        &self,
        drag_state: ChipDragState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        match self {
            Self::ContextChip(r) => r.render_unused(drag_state, appearance),
            Self::Control(r) => r.render_internal(drag_state, None, appearance),
        }
    }

    pub fn render_used<A: Action + Clone>(
        &self,
        drag_state: ChipDragState,
        on_remove_action: A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        match self {
            Self::ContextChip(r) => r.render_used(drag_state, on_remove_action, appearance),
            Self::Control(r) => {
                let remove_button = if r.removable {
                    Some(r.render_remove_button(on_remove_action, appearance))
                } else {
                    None
                };
                r.render_internal(drag_state, remove_button, appearance)
            }
        }
    }
}

/// Lightweight renderer for non-chip control items (model selector, NLD toggle,
/// voice input, image attach, file explorer, view changes, compose, etc.)
/// inside the configurator.
pub struct ControlItemRenderer {
    kind: Option<AgentToolbarItemKind>,
    custom_label: Option<String>,
    custom_icon: Option<crate::ui_components::icons::Icon>,
    /// An opaque string identifier for round-tripping items through the configurator.
    /// Used by the header toolbar editor to recover the `HeaderToolbarItemKind`.
    identifier: Option<String>,
    removable: bool,
    draggable_state: DraggableState,
    tooltip_state_handle: MouseStateHandle,
    remove_button_state_handle: MouseStateHandle,
}

impl ControlItemRenderer {
    pub fn new(kind: AgentToolbarItemKind) -> Self {
        Self {
            kind: Some(kind),
            custom_label: None,
            custom_icon: None,
            identifier: None,
            removable: true,
            draggable_state: Default::default(),
            tooltip_state_handle: Default::default(),
            remove_button_state_handle: Default::default(),
        }
    }

    pub fn new_with_label_and_icon(label: String, icon: crate::ui_components::icons::Icon) -> Self {
        Self {
            kind: None,
            custom_label: Some(label),
            custom_icon: Some(icon),
            identifier: None,
            removable: true,
            draggable_state: Default::default(),
            tooltip_state_handle: Default::default(),
            remove_button_state_handle: Default::default(),
        }
    }

    pub fn with_identifier(mut self, id: String) -> Self {
        self.identifier = Some(id);
        self
    }

    pub(crate) fn identifier(&self) -> Option<&str> {
        self.identifier.as_deref()
    }

    pub fn non_removable(mut self) -> Self {
        self.removable = false;
        self
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

    pub(crate) fn display_label(&self) -> &str {
        if let Some(label) = &self.custom_label {
            label
        } else if let Some(kind) = &self.kind {
            kind.display_label()
        } else {
            "Unknown"
        }
    }

    fn display_icon(&self) -> Option<crate::ui_components::icons::Icon> {
        if let Some(icon) = self.custom_icon {
            Some(icon)
        } else {
            self.kind.as_ref().and_then(|k| k.icon())
        }
    }

    fn render_internal(
        &self,
        drag_state: ChipDragState,
        remove_button: Option<Box<dyn Element>>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let font_size = udi_font_size(appearance);
        let label = self.display_label().to_string();
        let icon = self.display_icon();
        let is_dragging = matches!(drag_state, ChipDragState::Draggable { is_dragging: true });
        let mut hoverable = Hoverable::new(self.tooltip_state_handle.clone(), move |mouse_state| {
            let show_hover = mouse_state.is_hovered() && !is_dragging;
            let background = if show_hover {
                appearance.theme().surface_2()
            } else {
                appearance.theme().surface_1()
            };
            let color =
                crate::context_chips::readable_chip_label_color(appearance.theme(), background);

            let mut content = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

            if let Some(icon) = icon {
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
            let text = Text::new_inline(label.clone(), appearance.ui_font_family(), font_size)
                .with_color(color)
                .with_line_height_ratio(appearance.line_height_ratio())
                .with_style(Properties::default())
                .finish();
            content.add_child(text);

            if let Some(remove_button) = remove_button {
                content.add_child(
                    Container::new(remove_button)
                        .with_margin_left(spacing::UDI_CHIP_ICON_GAP)
                        .finish(),
                );
            }

            let button = chip_container(content.finish(), None, appearance)
                .with_background(background)
                .finish();
            if !show_hover {
                return button;
            }

            let tooltip = appearance.ui_builder().tool_tip(label.clone());
            let mut stack = Stack::new();
            stack.add_child(button);
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

        if matches!(drag_state, ChipDragState::Draggable { .. }) {
            hoverable = hoverable.with_cursor(Cursor::OpenHand);
        }

        hoverable.finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipLocation {
    Used { index: usize },
    Left { index: usize },
    Right { index: usize },
    Unused { index: usize },
}

impl ChipLocation {
    pub fn index(&self) -> usize {
        match self {
            Self::Used { index }
            | Self::Left { index }
            | Self::Right { index }
            | Self::Unused { index } => *index,
        }
    }

    pub fn in_same_area_as(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Used { .. }, Self::Used { .. })
                | (Self::Left { .. }, Self::Left { .. })
                | (Self::Right { .. }, Self::Right { .. })
                | (Self::Unused { .. }, Self::Unused { .. })
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CurrentDraggingState {
    pub original_location: ChipLocation,
    pub current_location: ChipLocation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChipConfiguratorAction {
    StartDraggingChip { location: ChipLocation },
    DragChip { current_position: RectF },
    DropChip { position: RectF },
    RemoveFromUsed { location: ChipLocation },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipConfiguratorLayout {
    SingleZone,
    LeftRightZones,
}

pub struct ChipConfigurator {
    layout: ChipConfiguratorLayout,

    pub used_chips: Vec<ConfigurableItem>,
    pub left_chips: Vec<ConfigurableItem>,
    pub right_chips: Vec<ConfigurableItem>,
    pub unused_chips: Vec<ConfigurableItem>,
    pub current_dragging_state: Option<CurrentDraggingState>,
}

impl ChipConfigurator {
    pub fn new(layout: ChipConfiguratorLayout) -> Self {
        Self {
            layout,
            used_chips: vec![],
            left_chips: vec![],
            right_chips: vec![],
            unused_chips: vec![],
            current_dragging_state: None,
        }
    }

    /// Initialize for `SingleZone` layout with pre-built context chip renderers.
    pub fn open_single_zone_with_renderers(
        &mut self,
        used_chips: Vec<ContextChipRenderer>,
        unused_chips: Vec<ContextChipRenderer>,
    ) {
        self.reset();
        self.used_chips = used_chips
            .into_iter()
            .map(|r| ConfigurableItem::ContextChip(Box::new(r)))
            .collect();
        self.unused_chips = unused_chips
            .into_iter()
            .map(|r| ConfigurableItem::ContextChip(Box::new(r)))
            .collect();
    }

    /// Initialize for `LeftRightZones` layout with `AgentToolbarItemKind` lists.
    pub fn open_left_right_zones_with_items(
        &mut self,
        left_items: Vec<AgentToolbarItemKind>,
        right_items: Vec<AgentToolbarItemKind>,
        available: Vec<AgentToolbarItemKind>,
        appearance: &Appearance,
    ) {
        self.reset();
        self.left_chips = left_items
            .iter()
            .filter_map(|kind| ConfigurableItem::from_toolbar_item(kind.clone(), appearance))
            .collect();
        self.right_chips = right_items
            .iter()
            .filter_map(|kind| ConfigurableItem::from_toolbar_item(kind.clone(), appearance))
            .collect();
        let used_set: Vec<_> = left_items
            .iter()
            .chain(right_items.iter())
            .cloned()
            .collect();
        self.unused_chips = available
            .into_iter()
            .filter_map(|kind| {
                (!used_set.contains(&kind))
                    .then(|| ConfigurableItem::from_toolbar_item(kind, appearance))
                    .flatten()
            })
            .collect();
    }

    pub fn reset(&mut self) {
        self.used_chips.clear();
        self.left_chips.clear();
        self.right_chips.clear();
        self.unused_chips.clear();
        self.current_dragging_state = None;
    }

    /// Whether any zone currently holds chips (i.e. the configurator is open /
    /// populated).
    pub fn has_items(&self) -> bool {
        !self.used_chips.is_empty()
            || !self.left_chips.is_empty()
            || !self.right_chips.is_empty()
            || !self.unused_chips.is_empty()
    }

    pub fn left_item_kinds(&self) -> Vec<AgentToolbarItemKind> {
        self.left_chips
            .iter()
            .filter_map(|r| r.item_kind())
            .collect()
    }

    pub fn right_item_kinds(&self) -> Vec<AgentToolbarItemKind> {
        self.right_chips
            .iter()
            .filter_map(|r| r.item_kind())
            .collect()
    }

    pub fn handle_action<V: View>(
        &mut self,
        action: &ChipConfiguratorAction,
        ctx: &mut ViewContext<V>,
    ) -> bool {
        match action {
            ChipConfiguratorAction::StartDraggingChip { location } => {
                self.current_dragging_state = Some(CurrentDraggingState {
                    original_location: *location,
                    current_location: *location,
                });
                false
            }
            ChipConfiguratorAction::DragChip { current_position } => {
                self.handle_chip_dragged(*current_position, ctx);
                false
            }
            ChipConfiguratorAction::DropChip { position } => {
                self.handle_chip_dropped(*position, ctx);
                true
            }
            ChipConfiguratorAction::RemoveFromUsed { location } => {
                if !self.is_chip_at_location_removable(*location) {
                    return false;
                }
                let unused_len = self.unused_chips.len();
                self.remove_and_insert_chip_at_location(
                    *location,
                    ChipLocation::Unused { index: unused_len },
                );
                true
            }
        }
    }

    fn is_chip_at_location_removable(&self, location: ChipLocation) -> bool {
        let chips = self.chips_for_location(location);
        let index = location.index();
        if index >= chips.len() {
            return true;
        }
        chips[index].is_removable()
    }

    fn chips_for_location(&self, location: ChipLocation) -> &Vec<ConfigurableItem> {
        match location {
            ChipLocation::Used { .. } => &self.used_chips,
            ChipLocation::Left { .. } => &self.left_chips,
            ChipLocation::Right { .. } => &self.right_chips,
            ChipLocation::Unused { .. } => &self.unused_chips,
        }
    }

    fn chips_for_location_mut(&mut self, location: ChipLocation) -> &mut Vec<ConfigurableItem> {
        match location {
            ChipLocation::Used { .. } => &mut self.used_chips,
            ChipLocation::Left { .. } => &mut self.left_chips,
            ChipLocation::Right { .. } => &mut self.right_chips,
            ChipLocation::Unused { .. } => &mut self.unused_chips,
        }
    }

    fn remove_and_insert_chip_at_location(&mut self, from: ChipLocation, to: ChipLocation) {
        let from_vec = self.chips_for_location(from);
        if from_vec.is_empty() {
            return;
        }
        let index = from.index().min(from_vec.len() - 1);

        if from.in_same_area_as(&to) {
            let vec = self.chips_for_location_mut(from);
            let removed = vec.remove(index);
            let to_index = to.index().min(vec.len());
            vec.insert(to_index, removed);
        } else {
            let from_vec = self.chips_for_location_mut(from);
            let removed = from_vec.remove(index);
            let to_vec = self.chips_for_location_mut(to);
            let to_index = to.index().min(to_vec.len());
            to_vec.insert(to_index, removed);
        }
    }

    fn chip_save_position_id(location: ChipLocation) -> String {
        let area = match location {
            ChipLocation::Used { .. } => "u",
            ChipLocation::Left { .. } => "l",
            ChipLocation::Right { .. } => "r",
            ChipLocation::Unused { .. } => "n",
        };
        format!("chip_cfg_{area}_{}", location.index())
    }

    fn drop_target_position_id(location: ChipLocation) -> &'static str {
        match location {
            ChipLocation::Used { .. } => USED_CHIPS_POSITION_ID,
            ChipLocation::Left { .. } => LEFT_CHIPS_POSITION_ID,
            ChipLocation::Right { .. } => RIGHT_CHIPS_POSITION_ID,
            ChipLocation::Unused { .. } => UNUSED_CHIPS_POSITION_ID,
        }
    }

    fn find_new_location_for_index<V: View>(
        &self,
        current_location: ChipLocation,
        dragged_position: RectF,
        index: usize,
        is_last: bool,
        chip_index_to_location: &dyn Fn(usize) -> ChipLocation,
        ctx: &mut ViewContext<V>,
    ) -> Option<ChipLocation> {
        let chip_location = chip_index_to_location(index);
        let chip_position_id = Self::chip_save_position_id(chip_location);
        let chip_position = ctx.element_position_by_id(chip_position_id)?;

        let dragged_at_adequate_height = chip_position.min_y() <= dragged_position.center().y()
            && dragged_position.center().y() <= chip_position.max_y();
        if !dragged_at_adequate_height {
            return None;
        }

        let is_to_the_right_of_last_chip =
            is_last && dragged_position.max_x() > chip_position.max_x();
        if is_to_the_right_of_last_chip {
            return Some(chip_index_to_location(index + 1));
        }

        if !chip_position.intersects(dragged_position) {
            return None;
        }

        let should_compare_index = current_location.in_same_area_as(&chip_location);
        let dragged_before = dragged_position.min_x() < chip_position.min_x()
            && (!should_compare_index || current_location.index() > index);
        let dragged_after = dragged_position.max_x() > chip_position.max_x()
            && (!should_compare_index || current_location.index() < index);

        if dragged_before || dragged_after {
            return Some(chip_location);
        }

        None
    }

    fn find_new_location_in_zone<V: View>(
        &self,
        current_location: ChipLocation,
        dragged_position: RectF,
        chips_count: usize,
        chip_index_to_location: &dyn Fn(usize) -> ChipLocation,
        ctx: &mut ViewContext<V>,
    ) -> Option<ChipLocation> {
        if chips_count == 0 {
            let target_id = Self::drop_target_position_id(chip_index_to_location(0));
            if let Some(target_pos) = ctx.element_position_by_id(target_id) {
                if target_pos.intersects(dragged_position) {
                    return Some(chip_index_to_location(0));
                }
            }
            return None;
        }

        for index in 0..chips_count {
            let is_last = index == chips_count - 1;
            if let Some(new_location) = self.find_new_location_for_index(
                current_location,
                dragged_position,
                index,
                is_last,
                chip_index_to_location,
                ctx,
            ) {
                // The "past the last chip" heuristic matches any chip dragged
                // beyond the final item in this zone. When adjacent zones share
                // a Y range (e.g. left/right drop zones on the same row), that
                // heuristic false-positives for chips that have left this zone.
                // Verify the dragged center is still within the zone's drop
                // target before accepting an append match.
                if new_location.index() >= chips_count {
                    let target_id = Self::drop_target_position_id(new_location);
                    if let Some(target_pos) = ctx.element_position_by_id(target_id) {
                        let center_x = dragged_position.center().x();
                        if center_x < target_pos.min_x() || center_x > target_pos.max_x() {
                            return None;
                        }
                    }
                }
                return Some(new_location);
            }
        }

        None
    }

    fn find_in_used<V: View>(
        &self,
        location: ChipLocation,
        dragged_position: RectF,
        ctx: &mut ViewContext<V>,
    ) -> Option<ChipLocation> {
        self.find_new_location_in_zone(
            location,
            dragged_position,
            self.used_chips.len(),
            &|i| ChipLocation::Used { index: i },
            ctx,
        )
    }

    fn find_in_left<V: View>(
        &self,
        location: ChipLocation,
        dragged_position: RectF,
        ctx: &mut ViewContext<V>,
    ) -> Option<ChipLocation> {
        self.find_new_location_in_zone(
            location,
            dragged_position,
            self.left_chips.len(),
            &|i| ChipLocation::Left { index: i },
            ctx,
        )
    }

    fn find_in_right<V: View>(
        &self,
        location: ChipLocation,
        dragged_position: RectF,
        ctx: &mut ViewContext<V>,
    ) -> Option<ChipLocation> {
        self.find_new_location_in_zone(
            location,
            dragged_position,
            self.right_chips.len(),
            &|i| ChipLocation::Right { index: i },
            ctx,
        )
    }

    fn find_in_unused<V: View>(
        &self,
        location: ChipLocation,
        dragged_position: RectF,
        ctx: &mut ViewContext<V>,
    ) -> Option<ChipLocation> {
        self.find_new_location_in_zone(
            location,
            dragged_position,
            self.unused_chips.len(),
            &|i| ChipLocation::Unused { index: i },
            ctx,
        )
    }

    fn handle_chip_dragged<V: View>(&mut self, dragged_position: RectF, ctx: &mut ViewContext<V>) {
        // Use the tracked current location rather than the closure-captured location,
        // which may be stale after chips have been reordered mid-drag.
        let location = match self.current_dragging_state {
            Some(state) => state.current_location,
            None => return,
        };

        // Scan the current zone first so that within-zone reordering always
        // takes priority over cross-zone moves. Without this, a chip near
        // the boundary between two zones could match in the wrong zone first,
        // and the cross-zone suppression guard would then block the move
        // entirely — preventing the within-zone reorder from ever running.
        let new_location = match self.layout {
            ChipConfiguratorLayout::SingleZone => match location {
                ChipLocation::Unused { .. } => self
                    .find_in_unused(location, dragged_position, ctx)
                    .or_else(|| self.find_in_used(location, dragged_position, ctx)),
                _ => self
                    .find_in_used(location, dragged_position, ctx)
                    .or_else(|| self.find_in_unused(location, dragged_position, ctx)),
            },
            ChipConfiguratorLayout::LeftRightZones => match location {
                ChipLocation::Right { .. } => self
                    .find_in_right(location, dragged_position, ctx)
                    .or_else(|| self.find_in_left(location, dragged_position, ctx))
                    .or_else(|| self.find_in_unused(location, dragged_position, ctx)),
                ChipLocation::Unused { .. } => self
                    .find_in_unused(location, dragged_position, ctx)
                    .or_else(|| self.find_in_left(location, dragged_position, ctx))
                    .or_else(|| self.find_in_right(location, dragged_position, ctx)),
                _ => self
                    .find_in_left(location, dragged_position, ctx)
                    .or_else(|| self.find_in_right(location, dragged_position, ctx))
                    .or_else(|| self.find_in_unused(location, dragged_position, ctx)),
            },
        };

        if let Some(new_location) = new_location {
            // Don't allow non-removable items to be dragged to the unused zone.
            if matches!(new_location, ChipLocation::Unused { .. })
                && !self.is_chip_at_location_removable(location)
            {
                return;
            }

            // Suppress cross-zone moves when the dragged center is still inside
            // the current zone's drop target. This prevents oscillation when
            // adding a chip to a zone causes a row wrap: the chip's layout
            // position lands on row 2, fails the height check on the next frame,
            // and the code erroneously moves it to an adjacent zone.
            //
            // We use center-point containment rather than rect intersection so
            // that horizontally-adjacent zones (left/right) allow moves once the
            // user's drag center crosses the boundary, even though the chip rect
            // still partially overlaps the old zone.
            if !new_location.in_same_area_as(&location) {
                let current_zone_id = Self::drop_target_position_id(location);
                if let Some(target_pos) = ctx.element_position_by_id(current_zone_id) {
                    let center = dragged_position.center();
                    let center_in_zone = center.x() >= target_pos.min_x()
                        && center.x() <= target_pos.max_x()
                        && center.y() >= target_pos.min_y()
                        && center.y() <= target_pos.max_y();
                    if center_in_zone {
                        return;
                    }
                }
            }

            self.remove_and_insert_chip_at_location(location, new_location);
            if let Some(state) = self.current_dragging_state.as_mut() {
                state.current_location = new_location;
            }
        }
    }

    fn handle_chip_dropped<V: View>(&mut self, drop_position: RectF, ctx: &mut ViewContext<V>) {
        if let Some(state) = &self.current_dragging_state {
            let expected = state.current_location;
            let target_id = Self::drop_target_position_id(expected);
            if let Some(target_pos) = ctx.element_position_by_id(target_id) {
                if !target_pos.intersects(drop_position) {
                    self.remove_and_insert_chip_at_location(
                        state.current_location,
                        state.original_location,
                    );
                }
            }
        }
        self.current_dragging_state = None;
    }

    pub fn render_draggable_chip<A: Action + Clone + 'static>(
        &self,
        location: ChipLocation,
        chip: Box<dyn Element>,
        item: &ConfigurableItem,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
    ) -> Box<dyn Element> {
        let clickable = EventHandler::new(chip)
            .on_left_mouse_down(move |ctx, _, _| {
                ctx.dispatch_typed_action(on_click_action.clone());
                DispatchEventResult::StopPropagation
            })
            .finish();

        let start_loc = location;

        SavePosition::new(
            Draggable::new(item.draggable_state(), clickable)
                .on_drag_start(move |ctx, _, _| {
                    ctx.dispatch_typed_action(wrap_chip_action(
                        ChipConfiguratorAction::StartDraggingChip {
                            location: start_loc,
                        },
                    ));
                })
                .on_drag(move |ctx, _, rect, _| {
                    ctx.dispatch_typed_action(wrap_chip_action(ChipConfiguratorAction::DragChip {
                        current_position: rect,
                    }));
                })
                .on_drop(move |ctx, _, position, _| {
                    ctx.dispatch_typed_action(wrap_chip_action(ChipConfiguratorAction::DropChip {
                        position,
                    }));
                })
                .finish(),
            &Self::chip_save_position_id(location),
        )
        .finish()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_wrappable_chips<'a, A: Action + Clone + 'static>(
        &self,
        chips: impl Iterator<Item = &'a ConfigurableItem>,
        location_fn: impl Fn(usize) -> ChipLocation,
        is_used: bool,
        on_click_action: A,
        on_remove_action_fn: impl Fn(usize) -> A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let drag_state = ChipDragState::Draggable {
            is_dragging: self.current_dragging_state.is_some(),
        };
        Wrap::row()
            .with_children(chips.enumerate().map(|(index, item)| {
                let location = location_fn(index);
                let rendered_chip = if is_used {
                    item.render_used(drag_state, on_remove_action_fn(index), appearance)
                } else {
                    item.render_unused(drag_state, appearance)
                };
                Container::new(self.render_draggable_chip(
                    location,
                    rendered_chip,
                    item,
                    on_click_action.clone(),
                    wrap_chip_action,
                ))
                .with_horizontal_margin(4.)
                .finish()
            }))
            .with_run_spacing(8.)
            .finish()
    }

    pub fn render_unused_chips_bank<A: Action + Clone + 'static>(
        &self,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let dummy = on_click_action.clone();
        SavePosition::new(
            self.render_wrappable_chips(
                self.unused_chips.iter(),
                |i| ChipLocation::Unused { index: i },
                false,
                on_click_action,
                move |_| dummy.clone(),
                wrap_chip_action,
                appearance,
            ),
            UNUSED_CHIPS_POSITION_ID,
        )
        .finish()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_drop_zone<A: Action + Clone + 'static>(
        &self,
        chips: &[ConfigurableItem],
        location_fn: impl Fn(usize) -> ChipLocation,
        remove_location_fn: impl Fn(usize) -> ChipLocation,
        position_id: &str,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let drag_state = ChipDragState::Draggable {
            is_dragging: self.current_dragging_state.is_some(),
        };

        let inner = if chips.is_empty() {
            Empty::new().finish()
        } else {
            Wrap::row()
                .with_children(chips.iter().enumerate().map(|(index, item)| {
                    let location = location_fn(index);
                    let remove_loc = remove_location_fn(index);
                    let rendered = item.render_used(
                        drag_state,
                        wrap_chip_action(ChipConfiguratorAction::RemoveFromUsed {
                            location: remove_loc,
                        }),
                        appearance,
                    );
                    Container::new(self.render_draggable_chip(
                        location,
                        rendered,
                        item,
                        on_click_action.clone(),
                        wrap_chip_action,
                    ))
                    .with_horizontal_margin(4.)
                    .finish()
                }))
                .with_run_spacing(8.)
                .finish()
        };

        SavePosition::new(
            Container::new(
                ConstrainedBox::new(inner)
                    .with_min_height(appearance.monospace_font_size() * 2.5)
                    .finish(),
            )
            .with_uniform_padding(8.)
            .with_background(appearance.theme().surface_1())
            .with_border(
                Border::all(2.)
                    .with_border_fill(appearance.theme().outline())
                    .with_dashed_border(Dash {
                        dash_length: 8.,
                        gap_length: 8.,
                        ..Default::default()
                    }),
            )
            .finish(),
            position_id,
        )
        .finish()
    }

    pub fn render_used_drop_zone<A: Action + Clone + 'static>(
        &self,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        self.render_drop_zone(
            &self.used_chips,
            |i| ChipLocation::Used { index: i },
            |i| ChipLocation::Used { index: i },
            USED_CHIPS_POSITION_ID,
            on_click_action,
            wrap_chip_action,
            appearance,
        )
    }

    pub fn render_left_drop_zone<A: Action + Clone + 'static>(
        &self,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        self.render_drop_zone(
            &self.left_chips,
            |i| ChipLocation::Left { index: i },
            |i| ChipLocation::Left { index: i },
            LEFT_CHIPS_POSITION_ID,
            on_click_action,
            wrap_chip_action,
            appearance,
        )
    }

    pub fn render_right_drop_zone<A: Action + Clone + 'static>(
        &self,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        self.render_drop_zone(
            &self.right_chips,
            |i| ChipLocation::Right { index: i },
            |i| ChipLocation::Right { index: i },
            RIGHT_CHIPS_POSITION_ID,
            on_click_action,
            wrap_chip_action,
            appearance,
        )
    }
}
