use std::collections::{HashMap, HashSet};
use std::mem;

use instant::Instant;
use itertools::{Either, Itertools};
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::AnsiColors;
use warpui::event::ScrollPhase;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::Vector2F;
use warpui::{AppContext, EntityId, SingletonEntity, ViewContext, ViewHandle};

use super::vertical_tabs::{space_chip_position_id, space_display_name};
use super::{Workspace, group_member_indices};
use crate::app_state::{SpaceSnapshot, TabGroupSnapshot, TabSnapshot, WindowSnapshot};
use crate::appearance::Appearance;
use crate::menu::{MenuItem, MenuItemFields};
use crate::pane_group::PaneGroup;
use crate::tab::{
    ColorPickerTarget, SelectedTabColor, TabData, color_picker_menu_items, uses_vertical_tabs,
};
use crate::themes::theme::AnsiColorIdentifier;
use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;
use crate::workspace::WorkspaceAction;
use crate::workspace::space::{Space, SpaceId, StashedSpace};
use crate::workspace::swipe_detector::SwipeDirection;
use crate::workspace::tab_group::{TabGroup, TabGroupId};

/// What a drag over a space switcher chip moves into that space when dropped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SpaceChipDrop {
    /// The tab with this pane group.
    Tab(EntityId),
    Group(TabGroupId),
}

/// What a "Move to space" sidecar item moves.
#[derive(Clone, Copy)]
enum MoveToSpaceSource {
    Tab(usize),
    Group(TabGroupId),
    Selection,
}

impl MoveToSpaceSource {
    fn action(self, space_id: Option<SpaceId>) -> WorkspaceAction {
        match self {
            Self::Tab(tab_index) => WorkspaceAction::MoveTabToSpace {
                tab_index,
                space_id,
            },
            Self::Group(group_id) => WorkspaceAction::MoveTabGroupToSpace { group_id, space_id },
            Self::Selection => WorkspaceAction::MoveSelectedTabsToSpace { space_id },
        }
    }
}

impl Workspace {
    /// All tabs in the window in switcher order, each with its space and its index in that space.
    pub(crate) fn all_tabs(&self) -> impl Iterator<Item = (SpaceId, usize, &TabData)> {
        self.spaces.iter().flat_map(|space| {
            let tabs = if space.id == self.active_space {
                self.tabs.as_slice()
            } else {
                self.stashed_spaces
                    .get(&space.id)
                    .map_or(&[][..], |stash| stash.tabs.as_slice())
            };
            tabs.iter()
                .enumerate()
                .map(|(index, tab)| (space.id, index, tab))
        })
    }

    /// Pane groups of every tab in the window, across every space.
    pub(crate) fn all_tab_views(&self) -> impl Iterator<Item = &ViewHandle<PaneGroup>> {
        self.all_tabs().map(|(.., tab)| &tab.pane_group)
    }

    /// Every tab in the window in Ctrl+Tab order, each with its index in its space and, when the
    /// window has more than one space, its space's display name. The active space's tabs come
    /// first, then each hidden space's in switcher order; each space's tabs are ordered by its
    /// own most-recently-used order.
    pub(super) fn tabs_in_mru_order(&self) -> Vec<(Option<String>, usize, &TabData)> {
        let label_spaces = FeatureFlag::Spaces.is_enabled() && self.spaces.len() > 1;
        let active_space_index = self.active_space_index();
        let space_order = std::iter::once(active_space_index)
            .chain((0..self.spaces.len()).filter(|index| *index != active_space_index));
        space_order
            .filter_map(|space_index| {
                let space = self.spaces.get(space_index)?;
                let (tabs, mru_order) = if space.id == self.active_space {
                    (self.tabs.as_slice(), self.tab_mru_order.as_slice())
                } else {
                    let stash = self.stashed_spaces.get(&space.id)?;
                    (stash.tabs.as_slice(), stash.tab_mru_order.as_slice())
                };
                let label = label_spaces.then(|| space_display_name(space, space_index + 1));
                Some(mru_order.iter().filter_map(move |pane_group_id| {
                    let (index, tab) = tabs
                        .iter()
                        .find_position(|tab| tab.pane_group.id() == *pane_group_id)?;
                    Some((label.clone(), index, tab))
                }))
            })
            .flatten()
            .collect()
    }

    /// Returns the live index of the tab whose pane group is `pane_group_id`. A tab in a hidden
    /// space becomes that space's active tab and the space is switched to first, so jumps from
    /// outside the tab list (palette, notifications, conversation links) can land on it.
    pub(super) fn reveal_tab_by_pane_group_id(
        &mut self,
        pane_group_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> Option<usize> {
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.pane_group.id() == pane_group_id)
        {
            return Some(index);
        }
        let (space_id, index) = self.all_tabs().find_map(|(space_id, index, tab)| {
            (tab.pane_group.id() == pane_group_id).then_some((space_id, index))
        })?;
        if let Some(stash) = self.stashed_spaces.get_mut(&space_id) {
            stash.active_tab_index = index;
        }
        self.switch_space(space_id, ctx);
        (self.active_space == space_id).then_some(index)
    }

    /// All tab groups in the window, across every space.
    pub(crate) fn all_tab_groups(&self) -> impl Iterator<Item = &TabGroup> {
        self.tab_groups.values().chain(
            self.stashed_spaces
                .values()
                .flat_map(|stash| stash.tab_groups.values()),
        )
    }

    pub(crate) fn active_space_index(&self) -> usize {
        self.spaces
            .iter()
            .position(|space| space.id == self.active_space)
            .unwrap_or_default()
    }

    /// Whether a space other than the active one has tabs, in which case closing the active
    /// space's last tab switches spaces instead of closing the window.
    pub(super) fn hidden_spaces_have_tabs(&self) -> bool {
        FeatureFlag::Spaces.is_enabled()
            && self
                .stashed_spaces
                .values()
                .any(|stash| !stash.tabs.is_empty())
    }

    /// The closest space that has tabs, preferring the earlier one at equal distance.
    fn nearest_non_empty_space(&self) -> Option<SpaceId> {
        let active_index = self.active_space_index();
        let space_with_tabs = |index: usize| {
            self.spaces.get(index).filter(|space| {
                self.stashed_spaces
                    .get(&space.id)
                    .is_some_and(|stash| !stash.tabs.is_empty())
            })
        };
        (1..self.spaces.len())
            .find_map(|distance| {
                active_index
                    .checked_sub(distance)
                    .and_then(space_with_tabs)
                    .or_else(|| space_with_tabs(active_index + distance))
            })
            .map(|space| space.id)
    }

    /// Puts `incoming` in the live tab fields and returns what they held.
    fn replace_live_tabs(&mut self, incoming: StashedSpace) -> StashedSpace {
        StashedSpace {
            tabs: mem::replace(&mut self.tabs, incoming.tabs),
            active_tab_index: mem::replace(&mut self.active_tab_index, incoming.active_tab_index),
            tab_mru_order: mem::replace(&mut self.tab_mru_order, incoming.tab_mru_order),
            tab_groups: mem::replace(&mut self.tab_groups, incoming.tab_groups),
        }
    }

    /// Adds `tabs` to the end of the space `space_id`, whether it is live or hidden.
    fn append_to_space(&mut self, space_id: SpaceId, tabs: StashedSpace) {
        if space_id == self.active_space {
            let mut live = self.replace_live_tabs(StashedSpace::default());
            live.append(tabs);
            self.replace_live_tabs(live);
        } else {
            self.stashed_spaces
                .entry(space_id)
                .or_default()
                .append(tabs);
        }
    }

    /// Clears UI state keyed by index into the live tab list, before that list changes.
    fn clear_index_keyed_tab_ui_state(&mut self, ctx: &mut ViewContext<Self>) {
        self.cancel_tab_rename(ctx);
        self.cancel_pane_rename(ctx);
        self.cancel_tab_group_rename(ctx);
        self.show_tab_right_click_menu = None;
        self.show_tab_group_right_click_menu = None;
        self.show_tab_selection_right_click_menu = None;
        self.hide_move_to_group_sidecar(ctx);
        self.hovered_tab_index = None;
        self.vertical_tabs_panel.clear_detail_sidecar();
        self.clear_tab_multi_selection(ctx);
    }

    /// Shows `target`'s tabs in place of the active space's. Hidden tabs keep their views (and
    /// sessions) alive. Switching to a space with no tabs opens a new tab in it.
    pub(crate) fn switch_space(&mut self, target: SpaceId, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::Spaces.is_enabled()
            || target == self.active_space
            || !self.spaces.iter().any(|space| space.id == target)
        {
            return;
        }

        self.clear_index_keyed_tab_ui_state(ctx);

        let from_index = self.active_space_index();
        let incoming = self.stashed_spaces.remove(&target).unwrap_or_default();
        let outgoing = self.replace_live_tabs(incoming);
        self.stashed_spaces.insert(self.active_space, outgoing);
        self.active_space = target;
        if uses_vertical_tabs(ctx) && self.vertical_tabs_panel_open {
            self.vertical_tabs_panel
                .start_space_slide(from_index, self.active_space_index());
        }

        if self.tabs.is_empty() {
            // Not `AddDefaultTab`: some default session modes (e.g. a tab config with params)
            // open a modal that reads the active tab instead of adding one synchronously, and
            // the live tab list must not stay empty.
            self.add_terminal_tab(false, ctx);
        } else {
            let index = self.active_tab_index.min(self.tabs.len() - 1);
            self.activate_tab(index, ctx);
        }
    }

    /// Switches to the space at the 1-based `number`, if it exists.
    pub(super) fn switch_to_space_number(&mut self, number: usize, ctx: &mut ViewContext<Self>) {
        if let Some(space) = number
            .checked_sub(1)
            .and_then(|index| self.spaces.get(index))
        {
            self.switch_space(space.id, ctx);
        }
    }

    /// Switches to the space `offset` positions away from the active one, without wrapping.
    pub(super) fn switch_to_adjacent_space(&mut self, offset: isize, ctx: &mut ViewContext<Self>) {
        if let Some(space) = self
            .active_space_index()
            .checked_add_signed(offset)
            .and_then(|index| self.spaces.get(index))
        {
            self.switch_space(space.id, ctx);
        }
    }

    /// Feeds a scroll event over the vertical tabs panel to the swipe detector and switches to the
    /// adjacent space when it completes a horizontal swipe. A swipe is ignored while a tab or group
    /// is dragged, something is renamed, or the panel's search field has focus; its events are
    /// still fed, so the rest of that gesture can't switch once the blocking state ends.
    pub(super) fn handle_space_swipe(
        &mut self,
        delta: Vector2F,
        precise: bool,
        phase: Option<ScrollPhase>,
        now: Instant,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::Spaces.is_enabled() {
            return;
        }
        let Some(direction) =
            self.space_swipe_detector
                .feed(delta.x(), delta.y(), precise, phase, now)
        else {
            return;
        };
        if self.blocks_space_swipes(ctx) {
            return;
        }
        let offset = match direction {
            SwipeDirection::Next => 1,
            SwipeDirection::Prev => -1,
        };
        self.switch_to_adjacent_space(offset, ctx);
        ctx.dispatch_global_action("workspace:save_app", ());
    }

    fn blocks_space_swipes(&self, app: &AppContext) -> bool {
        let state = &self.current_workspace_state;
        state.is_tab_being_dragged
            || self
                .tab_groups
                .values()
                .any(|group| group.draggable_state.is_dragging())
            || state.is_tab_being_renamed()
            || state.is_any_pane_being_renamed()
            || state.is_any_tab_group_being_renamed()
            || state.space_being_renamed().is_some()
            || self.vertical_tabs_panel.is_space_chip_dragging()
            || self.vertical_tabs_search_input.is_focused(app)
    }

    /// Switches to the closest space that has tabs. For when the active space has just lost its
    /// last tab.
    pub(super) fn switch_to_nearest_non_empty_space(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(target) = self.nearest_non_empty_space() else {
            // The live tab list must not stay empty.
            self.add_terminal_tab(false, ctx);
            return;
        };
        // Cancelling a rename refocuses the active tab, and the active space has none left.
        self.current_workspace_state.clear_tab_being_renamed();
        self.current_workspace_state.clear_pane_being_renamed();
        self.current_workspace_state.clear_tab_group_being_renamed();
        self.switch_space(target, ctx);
    }

    pub(super) fn create_and_switch_to_new_space(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::Spaces.is_enabled() {
            return;
        }
        let space_id = self.push_new_space();
        self.switch_space(space_id, ctx);
    }

    /// Adds an empty space after the existing ones, without switching to it.
    fn push_new_space(&mut self) -> SpaceId {
        // The first space keeps the neutral color; later ones cycle through the palette so they
        // are distinguishable before the user picks colors.
        let color = TAB_COLOR_OPTIONS[(self.spaces.len() - 1) % TAB_COLOR_OPTIONS.len()];
        let space = Space {
            color: SelectedTabColor::Color(color),
            ..Space::new()
        };
        let space_id = space.id;
        self.spaces.push(space);
        space_id
    }

    /// Moves a dragged switcher chip one slot whenever its center passes the midpoint of a
    /// neighboring chip, like tab drags. The placeholder name of an unnamed space depends on its
    /// position, so a space that moves keeps the name it was shown with.
    pub(super) fn on_space_chip_drag(
        &mut self,
        space_id: SpaceId,
        position: RectF,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::Spaces.is_enabled() {
            return;
        }
        let Some(index) = self.spaces.iter().position(|space| space.id == space_id) else {
            return;
        };
        let chip_rect = |index: usize| {
            let space = self.spaces.get(index)?;
            ctx.element_position_by_id(space_chip_position_id(space.id))
        };
        let target = space_chip_drag_index(
            index,
            position.center(),
            index.checked_sub(1).and_then(chip_rect),
            chip_rect(index + 1),
        );
        if target == index {
            return;
        }
        for slot in [index, target] {
            let space = &mut self.spaces[slot];
            if space.name.is_none() {
                space.name = Some(space_display_name(space, slot + 1));
            }
        }
        self.spaces.swap(index, target);
        ctx.notify();
    }

    /// The switcher chip under `cursor`, unless it is the active space's. Chip positions outlive
    /// the panel, so this is `None` while the panel isn't shown.
    fn space_chip_at(&self, cursor: Vector2F, ctx: &ViewContext<Self>) -> Option<SpaceId> {
        if !FeatureFlag::Spaces.is_enabled()
            || !uses_vertical_tabs(ctx)
            || !self.vertical_tabs_panel_open
        {
            return None;
        }
        self.spaces
            .iter()
            .filter(|space| space.id != self.active_space)
            .find(|space| {
                ctx.element_position_by_id(space_chip_position_id(space.id))
                    .is_some_and(|rect| rect.contains_point(cursor))
            })
            .map(|space| space.id)
    }

    /// Highlights the switcher chip under the cursor of a tab or group drag, as the space `source`
    /// moves to when dropped. Returns whether the cursor is over such a chip, in which case the
    /// drag shouldn't also reorder tabs.
    pub(super) fn update_space_chip_drop(
        &mut self,
        cursor: Option<Vector2F>,
        source: SpaceChipDrop,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let target = cursor
            .and_then(|cursor| self.space_chip_at(cursor, ctx))
            .map(|space_id| (space_id, source));
        if target != self.space_chip_drop {
            self.space_chip_drop = target;
            ctx.notify();
        }
        target.is_some()
    }

    /// Moves a tab or group dropped on a switcher chip to that space, like "Move to space".
    pub(super) fn drop_on_space_chip(&mut self, ctx: &mut ViewContext<Self>) {
        let Some((space_id, source)) = self.space_chip_drop.take() else {
            return;
        };
        match source {
            SpaceChipDrop::Tab(pane_group_id) => {
                if let Some(index) = self
                    .tabs
                    .iter()
                    .position(|tab| tab.pane_group.id() == pane_group_id)
                {
                    self.move_tab_to_space(index, Some(space_id), ctx);
                }
            }
            SpaceChipDrop::Group(group_id) => {
                self.move_tab_group_to_space(group_id, Some(space_id), ctx)
            }
        }
        ctx.notify();
    }

    /// Opens the inline rename editor over the space's switcher chip. The chip is only shown in
    /// the vertical tabs panel, so this is a no-op while that panel is hidden.
    pub(super) fn rename_space(&mut self, space_id: SpaceId, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::Spaces.is_enabled()
            || !uses_vertical_tabs(ctx)
            || !self.vertical_tabs_panel_open
        {
            return;
        }
        let Some((index, space)) = self
            .spaces
            .iter()
            .find_position(|space| space.id == space_id)
        else {
            return;
        };
        let seed_text = space_display_name(space, index + 1);

        self.current_workspace_state
            .set_space_being_renamed(space_id);
        self.clear_tab_group_name_editor(ctx);
        self.tab_group_rename_editor
            .update(ctx, move |editor, ctx| {
                editor.insert_selected_text(&seed_text, ctx);
            });
        ctx.focus(&self.tab_group_rename_editor);
        ctx.notify();
    }

    pub(super) fn finish_space_rename(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(space_id) = self.current_workspace_state.space_being_renamed() else {
            return;
        };
        self.current_workspace_state.clear_space_being_renamed();
        let title = self.tab_group_rename_editor.as_ref(ctx).buffer_text(ctx);
        let trimmed = title.trim();
        // An empty name keeps the current one, as with tab group renames.
        if !trimmed.is_empty()
            && let Some(space) = self.spaces.iter_mut().find(|space| space.id == space_id)
        {
            space.name = Some(trimmed.to_string());
        }
        self.clear_tab_group_name_editor(ctx);
        self.focus_active_tab(ctx);
        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    pub(super) fn cancel_space_rename(&mut self, ctx: &mut ViewContext<Self>) {
        if self.current_workspace_state.space_being_renamed().is_none() {
            return;
        }
        self.current_workspace_state.clear_space_being_renamed();
        self.clear_tab_group_name_editor(ctx);
        self.focus_active_tab(ctx);
        ctx.notify();
    }

    /// Sets the space's color, or clears it when `color` is already the space's color.
    pub(super) fn toggle_space_color(
        &mut self,
        space_id: SpaceId,
        color: AnsiColorIdentifier,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::Spaces.is_enabled() {
            return;
        }
        let Some(space) = self.spaces.iter_mut().find(|space| space.id == space_id) else {
            return;
        };
        space.color = if space.color.resolve(None) == Some(color) {
            SelectedTabColor::Cleared
        } else {
            SelectedTabColor::Color(color)
        };
        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    /// Deletes the space and moves its tabs and groups, sessions intact, to the previous space
    /// (else the next one). Deleting the active space switches to that neighbor. The last
    /// remaining space can't be deleted.
    pub(super) fn delete_space(&mut self, space_id: SpaceId, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::Spaces.is_enabled() || self.spaces.len() < 2 {
            return;
        }
        let Some(index) = self.spaces.iter().position(|space| space.id == space_id) else {
            return;
        };
        let neighbor = self.spaces[if index > 0 { index - 1 } else { index + 1 }].id;
        let was_active = space_id == self.active_space;

        if self.current_workspace_state.space_being_renamed() == Some(space_id) {
            self.cancel_space_rename(ctx);
        }
        if was_active || neighbor == self.active_space {
            self.clear_index_keyed_tab_ui_state(ctx);
        }

        let deleted = if was_active {
            let incoming = self.stashed_spaces.remove(&neighbor).unwrap_or_default();
            self.active_space = neighbor;
            self.replace_live_tabs(incoming)
        } else {
            self.stashed_spaces.remove(&space_id).unwrap_or_default()
        };
        self.append_to_space(neighbor, deleted);
        self.spaces.remove(index);
        self.vertical_tabs_panel.remove_space_chip_state(space_id);

        if was_active {
            if self.tabs.is_empty() {
                self.add_terminal_tab(false, ctx);
            } else {
                let index = self.active_tab_index.min(self.tabs.len() - 1);
                self.activate_tab(index, ctx);
            }
        }
        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    pub(super) fn move_tab_to_space(
        &mut self,
        tab_index: usize,
        space_id: Option<SpaceId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.move_tabs_to_space(&[tab_index], space_id, ctx);
    }

    pub(super) fn move_tab_group_to_space(
        &mut self,
        group_id: TabGroupId,
        space_id: Option<SpaceId>,
        ctx: &mut ViewContext<Self>,
    ) {
        let indices = group_member_indices(&self.tabs, group_id).collect_vec();
        self.move_tabs_to_space(&indices, space_id, ctx);
    }

    pub(super) fn move_selected_tabs_to_space(
        &mut self,
        space_id: Option<SpaceId>,
        ctx: &mut ViewContext<Self>,
    ) {
        let indices = self.selected_tab_indices();
        self.move_tabs_to_space(&indices, space_id, ctx);
    }

    /// Moves the live tabs at `indices` to the end of another space, or of a new space when
    /// `space_id` is `None`, without switching spaces. If the active tab moves, its neighbor in
    /// the active space becomes active; if no tab is left, the window switches to the nearest
    /// space with tabs.
    fn move_tabs_to_space(
        &mut self,
        indices: &[usize],
        space_id: Option<SpaceId>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::Spaces.is_enabled()
            || indices.is_empty()
            || indices.iter().any(|index| *index >= self.tabs.len())
        {
            return;
        }
        let target = match space_id {
            Some(space_id)
                if space_id != self.active_space
                    && self.spaces.iter().any(|space| space.id == space_id) =>
            {
                space_id
            }
            Some(_) => return,
            None => self.push_new_space(),
        };

        self.clear_index_keyed_tab_ui_state(ctx);
        let moved_indices: HashSet<usize> = indices.iter().copied().collect();
        let old_active_index = self.active_tab_index;
        let active_pane_group_id = self
            .tabs
            .get(old_active_index)
            .map(|tab| tab.pane_group.id());

        let moved = self.take_live_tabs(&moved_indices);
        self.stashed_spaces.entry(target).or_default().append(moved);

        if self.tabs.is_empty() {
            self.switch_to_nearest_non_empty_space(ctx);
        } else if moved_indices.contains(&old_active_index) {
            // Like closing the active tab: activate the tab that followed it, else the new last.
            let kept_before_active = (0..old_active_index)
                .filter(|index| !moved_indices.contains(index))
                .count();
            self.activate_tab(kept_before_active.min(self.tabs.len() - 1), ctx);
        } else {
            self.restore_active_tab_index(active_pane_group_id);
        }
        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    /// Removes the live tabs at `indices`, in order. A group travels with its tabs only when all
    /// of its members move; a member that moves without the rest leaves its group, so a group
    /// never spans two spaces.
    fn take_live_tabs(&mut self, indices: &HashSet<usize>) -> StashedSpace {
        let (mut moved_tabs, kept_tabs): (Vec<TabData>, Vec<TabData>) = mem::take(&mut self.tabs)
            .into_iter()
            .enumerate()
            .partition_map(|(index, tab)| {
                if indices.contains(&index) {
                    Either::Left(tab)
                } else {
                    Either::Right(tab)
                }
            });
        self.tabs = kept_tabs;

        let mut moved_groups = HashMap::new();
        for tab in &mut moved_tabs {
            let Some(group_id) = tab.group_id else {
                continue;
            };
            if moved_groups.contains_key(&group_id) {
                continue;
            }
            if self.tabs.iter().any(|kept| kept.group_id == Some(group_id)) {
                tab.group_id = None;
                continue;
            }
            match self.tab_groups.remove(&group_id) {
                Some(group) => {
                    moved_groups.insert(group_id, group);
                }
                None => tab.group_id = None,
            }
        }

        let moved_ids: HashSet<EntityId> =
            moved_tabs.iter().map(|tab| tab.pane_group.id()).collect();
        let (moved_mru, kept_mru): (Vec<EntityId>, Vec<EntityId>) =
            mem::take(&mut self.tab_mru_order)
                .into_iter()
                .partition(|id| moved_ids.contains(id));
        self.tab_mru_order = kept_mru;

        StashedSpace {
            tabs: moved_tabs,
            active_tab_index: 0,
            tab_mru_order: moved_mru,
            tab_groups: moved_groups,
        }
    }

    pub(super) fn toggle_space_right_click_menu(
        &mut self,
        space_id: SpaceId,
        position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.show_space_right_click_menu.is_some() {
            self.show_space_right_click_menu = None;
            ctx.notify();
            return;
        }
        let terminal_colors = Appearance::as_ref(ctx).theme().terminal_colors().normal;
        let menu_items = self.space_menu_items(space_id, terminal_colors);
        if menu_items.is_empty() {
            return;
        }
        self.tab_right_click_menu.update(ctx, |menu, ctx| {
            menu.set_items(menu_items, ctx);
        });
        self.show_tab_right_click_menu = None;
        self.show_tab_group_right_click_menu = None;
        self.show_tab_selection_right_click_menu = None;
        self.hide_move_to_group_sidecar(ctx);
        self.show_space_right_click_menu = Some(position);
        ctx.focus(&self.tab_right_click_menu);
        ctx.notify();
    }

    /// Items of a switcher chip's right-click menu. "Delete space" is hidden for the last space.
    fn space_menu_items(
        &self,
        space_id: SpaceId,
        terminal_colors: AnsiColors,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        if !FeatureFlag::Spaces.is_enabled() {
            return vec![];
        }
        let Some(space) = self.spaces.iter().find(|space| space.id == space_id) else {
            return vec![];
        };
        let mut menu_items = vec![
            MenuItemFields::new("Rename space")
                .with_on_select_action(WorkspaceAction::RenameSpace(space_id))
                .into_item(),
            MenuItem::Separator,
        ];
        menu_items.extend(color_picker_menu_items(
            space.color.resolve(None),
            terminal_colors,
            ColorPickerTarget::Space { space_id },
        ));
        if self.spaces.len() > 1 {
            menu_items.push(MenuItem::Separator);
            menu_items.push(
                MenuItemFields::new("Delete space")
                    .with_on_select_action(WorkspaceAction::DeleteSpace(space_id))
                    .into_item(),
            );
        }
        menu_items
    }

    /// Items of the "Move to space" sidecar for whichever tab, group, or selection menu is open:
    /// the other spaces in switcher order, then "New space".
    pub(super) fn build_move_to_space_sidecar_items(&self) -> Vec<MenuItem<WorkspaceAction>> {
        let source = if let Some((tab_index, _)) = self.show_tab_right_click_menu {
            MoveToSpaceSource::Tab(tab_index)
        } else if let Some((group_id, _)) = self.show_tab_group_right_click_menu {
            MoveToSpaceSource::Group(group_id)
        } else if self.show_tab_selection_right_click_menu.is_some() {
            MoveToSpaceSource::Selection
        } else {
            return vec![];
        };

        let mut menu_items = self
            .spaces
            .iter()
            .enumerate()
            .filter(|(_, space)| space.id != self.active_space)
            .map(|(index, space)| {
                MenuItemFields::new(space_display_name(space, index + 1))
                    .with_on_select_action(source.action(Some(space.id)))
                    .into_item()
            })
            .collect_vec();
        if !menu_items.is_empty() {
            menu_items.push(MenuItem::Separator);
        }
        menu_items.push(
            MenuItemFields::new("New space")
                .with_on_select_action(source.action(None))
                .into_item(),
        );
        menu_items
    }

    /// Puts the tabs saved by `snapshot` in their persisted order and snapshots the spaces.
    /// `saved_tabs` are `all_tabs` minus the tabs left out of the snapshot. Pinned tabs of every
    /// space come first, so a restore that ignores spaces still gets pinned tabs at the front,
    /// while each space keeps its own tab order. Returns the ordered tabs, the index among them of
    /// the window's active tab, and the spaces (none while the feature is off).
    pub(super) fn order_snapshot_tabs(
        &self,
        saved_tabs: Vec<(SpaceId, usize, TabSnapshot)>,
        tab_groups: &[TabGroupSnapshot],
    ) -> (Vec<TabSnapshot>, usize, Vec<SpaceSnapshot>) {
        let is_pinned = |tab: &TabSnapshot| {
            tab.pinned
                || tab.group_id.is_some_and(|group_id| {
                    tab_groups
                        .iter()
                        .any(|group| group.id == group_id && group.pinned)
                })
        };
        let (pinned, unpinned): (Vec<_>, Vec<_>) = saved_tabs
            .into_iter()
            .partition(|(_, _, tab)| is_pinned(tab));
        let (origins, tabs): (Vec<(SpaceId, usize)>, Vec<TabSnapshot>) = pinned
            .into_iter()
            .chain(unpinned)
            .map(|(space_id, tab_index, tab)| ((space_id, tab_index), tab))
            .unzip();

        // Where a space's active tab ended up: its index in `tabs` and among the space's own saved
        // tabs. An active tab left out of the snapshot hands over to the next saved tab.
        let saved_active_tab = |space_id: SpaceId, active_tab_index: usize| {
            let space_tabs = origins
                .iter()
                .positions(|(tab_space, _)| *tab_space == space_id)
                .collect_vec();
            let index_in_space = origins
                .iter()
                .filter(|(tab_space, tab_index)| {
                    *tab_space == space_id && *tab_index < active_tab_index
                })
                .count()
                .min(space_tabs.len().checked_sub(1)?);
            Some((space_tabs[index_in_space], index_in_space))
        };

        let active_tab_index = saved_active_tab(self.active_space, self.active_tab_index)
            .map_or(0, |(index, _)| index);
        let spaces = if FeatureFlag::Spaces.is_enabled() {
            self.spaces
                .iter()
                .map(|space| {
                    let space_active_tab_index = if space.id == self.active_space {
                        self.active_tab_index
                    } else {
                        self.stashed_spaces
                            .get(&space.id)
                            .map_or(0, |stash| stash.active_tab_index)
                    };
                    SpaceSnapshot {
                        id: space.id,
                        name: space.name.clone(),
                        color: space.color,
                        active_tab_index: saved_active_tab(space.id, space_active_tab_index)
                            .map_or(0, |(_, index_in_space)| index_in_space),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        (tabs, active_tab_index, spaces)
    }

    /// Moves the tabs just restored from `window_snapshot`, which are all live and in snapshot
    /// order, into the snapshot's spaces, and returns the live index of the tab to activate.
    /// Returns `None` and leaves every tab in the window's single space when the feature is off or
    /// the snapshot has no spaces. A tab whose space is unknown joins the first space. If the saved
    /// active space has no tabs, the nearest space with tabs is shown instead, as when the active
    /// space loses its last tab.
    pub(super) fn restore_spaces(&mut self, window_snapshot: &WindowSnapshot) -> Option<usize> {
        if !FeatureFlag::Spaces.is_enabled() || window_snapshot.spaces.is_empty() {
            return None;
        }
        self.spaces = window_snapshot
            .spaces
            .iter()
            .map(|space| Space {
                id: space.id,
                name: space.name.clone(),
                color: space.color,
            })
            .collect();
        let first_space = self.spaces[0].id;
        let resolve_space = |space_id: Option<SpaceId>| {
            space_id
                .filter(|space_id| self.spaces.iter().any(|space| space.id == *space_id))
                .unwrap_or(first_space)
        };
        self.active_space = resolve_space(window_snapshot.active_space);
        // Parallel to `self.tabs`, which stays in snapshot order as tabs are taken out below.
        let mut tab_spaces = (0..self.tabs.len())
            .map(|index| {
                resolve_space(window_snapshot.tabs.get(index).and_then(|tab| tab.space_id))
            })
            .collect_vec();

        let mut stashes = HashMap::new();
        for saved_space in &window_snapshot.spaces {
            let indices: HashSet<usize> = tab_spaces
                .iter()
                .positions(|space_id| *space_id == saved_space.id)
                .collect();
            if indices.is_empty() {
                continue;
            }
            tab_spaces.retain(|space_id| *space_id != saved_space.id);
            let mut stash = self.take_live_tabs(&indices);
            stash.active_tab_index = saved_space.active_tab_index.min(stash.tabs.len() - 1);
            stashes.insert(saved_space.id, stash);
        }
        self.replace_live_tabs(stashes.remove(&self.active_space).unwrap_or_default());
        self.stashed_spaces = stashes;

        if self.tabs.is_empty()
            && let Some(space_id) = self.nearest_non_empty_space()
        {
            let incoming = self.stashed_spaces.remove(&space_id).unwrap_or_default();
            self.replace_live_tabs(incoming);
            self.active_space = space_id;
        }
        Some(self.active_tab_index)
    }

    /// Removes the tab of an exited pane group from a hidden space. Hidden tabs aren't in
    /// `self.tabs`, so the regular close path can't find them.
    pub(super) fn remove_stashed_tab(
        &mut self,
        pane_group_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        let working_directories_model = self.working_directories_model.clone();
        for stash in self.stashed_spaces.values_mut() {
            let Some(index) = stash
                .tabs
                .iter()
                .position(|tab| tab.pane_group.id() == pane_group_id)
            else {
                continue;
            };
            let removed = stash.tabs.remove(index);
            removed.pane_group.update(ctx, |pane_group, ctx| {
                pane_group.detach_panes_for_close(&working_directories_model, ctx);
            });
            stash.tab_mru_order.retain(|id| *id != pane_group_id);
            if index < stash.active_tab_index || stash.active_tab_index >= stash.tabs.len() {
                stash.active_tab_index = stash.active_tab_index.saturating_sub(1);
            }
            if let Some(group_id) = removed.group_id
                && !stash.tabs.iter().any(|tab| tab.group_id == Some(group_id))
            {
                stash.tab_groups.remove(&group_id);
            }
            return;
        }
    }
}

/// Slot of a switcher chip dragged from `index` with its center at `center`: one earlier once the
/// center passes the midpoint of the chip `before` it, one later once it passes the midpoint of the
/// chip `after` it. Positions compare in reading order, so chips wrapped onto several rows reorder
/// too.
pub(super) fn space_chip_drag_index(
    index: usize,
    center: Vector2F,
    before: Option<RectF>,
    after: Option<RectF>,
) -> usize {
    let is_before = |rect: RectF| {
        center.y() < rect.min_y() || (center.y() <= rect.max_y() && center.x() < rect.center().x())
    };
    let is_after = |rect: RectF| {
        center.y() > rect.max_y() || (center.y() >= rect.min_y() && center.x() > rect.center().x())
    };
    if before.is_some_and(is_before) {
        index - 1
    } else if after.is_some_and(is_after) {
        index + 1
    } else {
        index
    }
}

#[cfg(test)]
#[path = "spaces_tests.rs"]
mod tests;
