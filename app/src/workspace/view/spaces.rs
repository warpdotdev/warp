use std::mem;

use warp_core::features::FeatureFlag;
use warpui::{EntityId, ViewContext};

use super::Workspace;
use crate::tab::{SelectedTabColor, TabData};
use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;
use crate::workspace::space::{Space, SpaceId, StashedSpace};
use crate::workspace::tab_group::TabGroup;

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

    /// Shows `target`'s tabs in place of the active space's. Hidden tabs keep their views (and
    /// sessions) alive. Switching to a space with no tabs opens a new tab in it.
    pub(crate) fn switch_space(&mut self, target: SpaceId, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::Spaces.is_enabled()
            || target == self.active_space
            || !self.spaces.iter().any(|space| space.id == target)
        {
            return;
        }

        // Transient UI state is keyed by index into the outgoing tab list.
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

        let incoming = self.stashed_spaces.remove(&target).unwrap_or_default();
        let outgoing = StashedSpace {
            tabs: mem::replace(&mut self.tabs, incoming.tabs),
            active_tab_index: mem::replace(&mut self.active_tab_index, incoming.active_tab_index),
            tab_mru_order: mem::replace(&mut self.tab_mru_order, incoming.tab_mru_order),
            tab_groups: mem::replace(&mut self.tab_groups, incoming.tab_groups),
        };
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

    pub(super) fn create_and_switch_to_new_space(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::Spaces.is_enabled() {
            return;
        }
        // The first space keeps the neutral color; later ones cycle through the palette so they
        // are distinguishable before the user picks colors.
        let color = TAB_COLOR_OPTIONS[(self.spaces.len() - 1) % TAB_COLOR_OPTIONS.len()];
        let space = Space {
            color: SelectedTabColor::Color(color),
            ..Space::new()
        };
        let space_id = space.id;
        self.spaces.push(space);
        self.switch_space(space_id, ctx);
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
