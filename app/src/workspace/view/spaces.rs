use std::collections::{HashMap, HashSet};
use std::mem;

use itertools::{Either, Itertools};
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::AnsiColors;
use warpui::geometry::vector::Vector2F;
use warpui::{EntityId, SingletonEntity, ViewContext};

use super::vertical_tabs::space_display_name;
use super::{Workspace, group_member_indices};
use crate::appearance::Appearance;
use crate::menu::{MenuItem, MenuItemFields};
use crate::tab::{
    ColorPickerTarget, SelectedTabColor, TabData, color_picker_menu_items, uses_vertical_tabs,
};
use crate::themes::theme::AnsiColorIdentifier;
use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;
use crate::workspace::WorkspaceAction;
use crate::workspace::space::{Space, SpaceId, StashedSpace};
use crate::workspace::tab_group::{TabGroup, TabGroupId};

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
    /// All tabs in the window: the active space's first, then hidden spaces' in display order.
    pub(crate) fn all_tabs(&self) -> impl Iterator<Item = (SpaceId, &TabData)> {
        let active_space = self.active_space;
        let active = self.tabs.iter().map(move |tab| (active_space, tab));
        let hidden = self.spaces.iter().flat_map(|space| {
            self.stashed_spaces
                .get(&space.id)
                .into_iter()
                .flat_map(|stash| stash.tabs.iter().map(|tab| (space.id, tab)))
        });
        active.chain(hidden)
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

        let incoming = self.stashed_spaces.remove(&target).unwrap_or_default();
        let outgoing = self.replace_live_tabs(incoming);
        self.stashed_spaces.insert(self.active_space, outgoing);
        self.active_space = target;

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

#[cfg(test)]
#[path = "spaces_tests.rs"]
mod tests;
