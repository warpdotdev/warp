use std::collections::HashSet;

use itertools::{Either, Itertools};
use warp_core::features::FeatureFlag;
use warpui::{EntityId, UpdateView, ViewContext};

use super::{group_member_indices, Workspace};
use crate::menu::{MenuItem, MenuItemFields};
use crate::tab::{TabData, MOVE_TO_GROUP_LABEL};
use crate::workspace::action::{TabContextMenuAnchor, WorkspaceAction};
use crate::workspace::tab_group::{TabGroup, TabGroupId};
use crate::workspace::util::PaneViewLocator;

// TODO(johnturcoo) move tab grouping helpers here from workspace/view.rs.
impl Workspace {
    /// Clears the multi-selection on every tab.
    pub(super) fn clear_tab_multi_selection(&mut self, ctx: &mut ViewContext<Self>) {
        for tab in &mut self.tabs {
            tab.in_multi_selection = false;
        }
        ctx.notify();
    }

    /// Adds the inclusive range between `anchor_index` and `clicked_index` to
    /// the multi-selection, expanding any collapsed groups the range crosses.
    /// Existing multi-selection outside the range is preserved (additive
    /// semantics), so cmd-click selections survive a subsequent shift-click.
    fn set_tab_range_selection(
        &mut self,
        anchor_index: usize,
        clicked_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        // Determine the bounds for our range selection.
        let lo_index = anchor_index.min(clicked_index);
        let hi_index = anchor_index.max(clicked_index);

        // Identify groups in the selection range.
        let crossed_group_ids: HashSet<TabGroupId> = self
            .tabs
            .get(lo_index..=hi_index)
            .into_iter()
            .flatten()
            .filter_map(|tab| tab.group_id)
            .collect();

        // Expand any groups within the selected range, so user can see what they are selecting.
        for group_id in &crossed_group_ids {
            self.expand_tab_group(*group_id, ctx);
        }

        // Add tabs in the selected range to the multi-selection.
        self.tabs
            .iter_mut()
            .enumerate()
            .filter(|(index, _)| (lo_index..=hi_index).contains(index))
            .for_each(|(_, tab)| tab.in_multi_selection = true);

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    /// Shift-click on a vertical tab row: selects every tab between the
    /// active tab and `locator` (inclusive).
    pub(super) fn shift_select_tab_range(
        &mut self,
        locator: PaneViewLocator,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::GroupedTabs.is_enabled() {
            return;
        }
        // Identify index of the tab that was shift-clicked.
        if let Some(clicked_index) = self
            .tabs
            .iter()
            .position(|tab| tab.pane_group.id() == locator.pane_group_id)
        {
            self.set_tab_range_selection(self.active_tab_index, clicked_index, ctx);
        }
    }

    /// Cmd-click on a tab: toggles the multi-selection flag
    /// for a single tab.
    pub(super) fn toggle_tab_multi_selection(
        &mut self,
        locator: PaneViewLocator,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::GroupedTabs.is_enabled() {
            return;
        }
        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.pane_group.id() == locator.pane_group_id)
        {
            // Toggle multi selection flag for this tab.
            tab.in_multi_selection = !tab.in_multi_selection;
            ctx.notify();
        }
    }

    /// Returns all tabs that are part of the multi tab selection.
    /// The active tab index is always included if any other tab is marked
    /// as selected. This is to handle the edge case where we only mark other
    /// tabs as selected via command click.
    pub(super) fn selected_tab_indices(&self) -> Vec<usize> {
        let any_flagged = self.tabs.iter().any(|tab| tab.in_multi_selection);
        // If no tab is part of the multi selection, return empty list.
        if !any_flagged {
            return Vec::new();
        }
        // Otherwise, the active tab must always be part of the multi tab selection.
        self.tabs
            .iter()
            .enumerate()
            .filter(|(index, tab)| tab.in_multi_selection || *index == self.active_tab_index)
            .map(|(index, _)| index)
            .collect()
    }

    /// Drives right-click menu dispatch: when a selected tab is right-clicked
    /// and the selection covers multiple tabs, show the multi-tab menu;
    /// otherwise fall through to the normal single-pane menu.
    pub(super) fn is_tab_in_multi_tab_selection(&self, tab_index: usize) -> bool {
        if !FeatureFlag::GroupedTabs.is_enabled() {
            return false;
        }
        let indices = self.selected_tab_indices();
        indices.len() > 1 && indices.contains(&tab_index)
    }

    /// Gates the "Remove from group" menu item. All selected tabs
    /// must be in the same group in order to display this option.
    pub(super) fn selection_shared_group(&self) -> Option<TabGroupId> {
        let indices = self.selected_tab_indices();
        let mut group_ids = indices
            .iter()
            .filter_map(|index| self.tabs.get(*index))
            .map(|tab| tab.group_id);
        let first = group_ids.next()??;
        group_ids.all(|gid| gid == Some(first)).then_some(first)
    }

    /// Re-seats `active_tab_index` so the previously-active pane group stays
    /// visually active across a tab reorder. Pass the pane group id captured
    /// before the reorder; no-op if it can't be found.
    pub(super) fn restore_active_tab_index(&mut self, pane_group_id: Option<EntityId>) {
        if let Some(active_id) = pane_group_id {
            if let Some(new_index) = self
                .tabs
                .iter()
                .position(|tab| tab.pane_group.id() == active_id)
            {
                self.active_tab_index = new_index;
            }
        }
    }

    /// Context-aware "create group" entry point used by the
    /// `workspace:new_tab_group_from_active_or_selected_tabs` keybinding. When
    /// the multi-selection covers 2+ tabs, groups the selection; otherwise
    /// groups just the active tab. `selected_tab_indices` already folds the
    /// active tab into the selection, so a lone flagged active tab (or no
    /// selection at all) takes the single-tab path.
    pub(super) fn new_tab_group_from_active_or_selected_tabs(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::GroupedTabs.is_enabled() {
            return;
        }
        if self.selected_tab_indices().len() >= 2 {
            self.new_tab_group_from_selected_tabs(ctx);
        } else {
            self.new_tab_group_from_tab(self.active_tab_index, ctx);
        }
    }

    /// Context-aware "remove from group" entry point used by the
    /// `workspace:remove_active_or_selected_tabs_from_group` keybinding. With a
    /// 2+ multi-selection, removes the whole selection from its group;
    /// otherwise removes just the active tab.
    pub(super) fn remove_active_or_selected_tabs_from_group(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::GroupedTabs.is_enabled() {
            return;
        }
        if self.selected_tab_indices().len() >= 2 {
            self.remove_selected_tabs_from_group(ctx);
        } else {
            self.remove_tab_from_group(self.active_tab_index, ctx);
        }
    }

    /// "Create group from tabs" menu action. Group membership requires
    /// tabs to be contiguous in the bar, so we gather the selected tabs into
    /// a single block anchored at the earliest selected tab's position before
    /// binding them to the new group. When that earliest tab was itself in a
    /// group, the block is placed just past that group's last remaining
    /// member so the existing group stays contiguous instead of being split.
    pub(super) fn new_tab_group_from_selected_tabs(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::GroupedTabs.is_enabled() {
            return;
        }
        let selected_indices = self.selected_tab_indices();

        // Should be unreachable: the multi-tab menu only opens when 2+ tabs
        // are selected.
        if selected_indices.len() < 2 {
            log::warn!(
                "new_tab_group_from_selected_tabs called with {} selected tab(s); expected at least 2",
                selected_indices.len()
            );
            return;
        }

        // Remember the groups the selected tabs are leaving so we can prune
        // any that become empty after the move.
        let previous_group_ids: HashSet<TabGroupId> = selected_indices
            .iter()
            .filter_map(|index| self.tabs[*index].group_id)
            .collect();

        let group = TabGroup::new();
        let group_id = group.id;
        self.tab_groups.insert(group_id, group);

        // Store the active tab (pane group).
        let active_pane_group_id = self
            .tabs
            .get(self.active_tab_index)
            .map(|tab| tab.pane_group.id());

        // Anchor the group block at the earliest selected tab. `selected_indices`
        // is ascending, so its first entry is the earliest tab in the list, and
        // we remember the group it currently belongs to (if any).
        let anchor_index = selected_indices[0];
        let anchor_previous_group_id = self.tabs[anchor_index].group_id;

        // Assign membership and clear flags for every selected tab. The new
        // group is unpinned, so any selected tab in set as unpinned.
        for &index in &selected_indices {
            let tab = &mut self.tabs[index];
            tab.group_id = Some(group_id);
            tab.pinned = false;
            tab.in_multi_selection = false;
        }

        // Split tabs into the new group's members and all other tabs.
        let (selected_tabs, mut other_tabs): (Vec<_>, Vec<_>) = self
            .tabs
            .drain(..)
            .partition(|tab| tab.group_id == Some(group_id));

        // Compute where to splice the new group block into `other_tabs`.
        //
        // Simple case — anchor tab was NOT in a group:
        //   Every tab before `anchor_index` in the original list was unselected
        //   (because `anchor_index` is the smallest selected index), so those
        //   tabs are still at the front of `other_tabs` in their original order.
        //   Inserting at `anchor_index` in `other_tabs` places the block exactly
        //   where the anchor tab used to be.
        //
        // Edge case — anchor tab WAS in an existing group G:
        //   Other surviving members of G are still in `other_tabs`. Inserting at
        //   `anchor_index` could land in the middle of G's run and split it.
        //   Example: tabs = [A(G), B(G), C(G), D] and we select B and D.
        //   other_tabs = [A(G), C(G), D]. anchor_index = 1, which points at C —
        //   inserting there would produce [A(G), B(new), D(new), C(G)], breaking
        //   G's contiguity. Instead we search other_tabs from the right for the
        //   last surviving G member (C, at index 1) and insert after it (index 2),
        //   giving [A(G), C(G), B(new), D(new)].
        let insert_at = anchor_previous_group_id
            .and_then(|prev_group_id| {
                other_tabs
                    .iter()
                    .rposition(|tab| tab.group_id == Some(prev_group_id))
                    .map(|last| last + 1)
            })
            .unwrap_or(anchor_index);

        // Our insertion index for this group should be below any pinned items.
        let insert_at = self.clamp_to_unpinned_region(&other_tabs, insert_at);

        other_tabs.splice(insert_at..insert_at, selected_tabs);
        self.tabs = other_tabs;

        self.restore_active_tab_index(active_pane_group_id);

        // Prune any groups that are now empty.
        for previous_group_id in previous_group_ids {
            self.prune_empty_tab_group(previous_group_id, ctx);
        }

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();

        ctx.dispatch_typed_action_deferred(WorkspaceAction::RenameTabGroup(group_id));
    }

    /// "Move to group" menu action. The destination group's first-member
    /// position is preserved so the group doesn't visually jump while the
    /// selected tabs are folded in.
    pub(super) fn move_selected_tabs_to_group(
        &mut self,
        group_id: TabGroupId,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::GroupedTabs.is_enabled() || !self.tab_groups.contains_key(&group_id) {
            return;
        }
        let selected_indices = self.selected_tab_indices();

        // Should be unreachable: the multi-tab menu only opens when 2+ tabs
        // are selected.
        if selected_indices.len() < 2 {
            log::warn!(
                "move_selected_tabs_to_group called with {} selected tab(s); expected at least 2",
                selected_indices.len()
            );
            return;
        }

        // Store all groups that tabs previously belonged to, excluding the
        // group that we are moving tabs to. In order to prune these groups later.
        let previous_group_ids: HashSet<TabGroupId> = selected_indices
            .iter()
            .filter_map(|index| self.tabs[*index].group_id)
            .filter(|gid| *gid != group_id)
            .collect();

        // Anchor the block at the existing first member so the group doesn't jump.
        let first_existing_member = self
            .tabs
            .iter()
            .position(|tab| tab.group_id == Some(group_id));

        // Store the active tab (pane group).
        let active_pane_group_id = self
            .tabs
            .get(self.active_tab_index)
            .map(|tab| tab.pane_group.id());

        // Assign membership and clear flags for every selected tab. Entering
        // the group removes any per-tab pinned flag — the destination group's
        // own `pinned` flag now governs the member's position.
        for &index in &selected_indices {
            let tab = &mut self.tabs[index];
            tab.group_id = Some(group_id);
            tab.pinned = false;
            tab.in_multi_selection = false;
        }

        // Anchor the group block at its original first-member position, shifted
        // left by the count of newly-added members from before that position.
        let insert_at = first_existing_member.map_or(0, |first| {
            first - selected_indices.iter().filter(|&&i| i < first).count()
        });
        // Split tabs into the destination group's members (existing + newly
        // added) and the rest.
        let (members, mut rest): (Vec<_>, Vec<_>) = self
            .tabs
            .drain(..)
            .partition(|tab| tab.group_id == Some(group_id));
        // Drop the group block into rest at the anchored position.
        rest.splice(insert_at..insert_at, members);
        self.tabs = rest;

        self.restore_active_tab_index(active_pane_group_id);

        self.expand_tab_group(group_id, ctx);

        // Prune any groups that are now empty.
        for previous_group_id in previous_group_ids {
            self.prune_empty_tab_group(previous_group_id, ctx);
        }

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    /// "Remove from group" menu action. Removed tabs land just below the
    /// group's remaining members so the user can still see where they came
    /// from; if the group ends up empty it's pruned and the removed block
    /// anchors at the original position instead.
    pub(super) fn remove_selected_tabs_from_group(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::GroupedTabs.is_enabled() {
            return;
        }
        let Some(group_id) = self.selection_shared_group() else {
            // Only a single-group selection has an unambiguous group to leave.
            self.clear_tab_multi_selection(ctx);
            return;
        };

        // Capture the group's first index before clearing membership so we can
        // anchor the removed block if the group ends up empty.
        let group_first_index = self
            .tabs
            .iter()
            .position(|tab| tab.group_id == Some(group_id))
            .unwrap_or(0);
        // Store the active tab (pane group).
        let active_pane_group_id = self
            .tabs
            .get(self.active_tab_index)
            .map(|tab| tab.pane_group.id());
        let selected_indices = self.selected_tab_indices();
        let selected_set: HashSet<usize> = selected_indices.iter().copied().collect();

        // Clear the group that all selected tabs belonged to.
        for &index in &selected_indices {
            self.tabs[index].group_id = None;
        }

        // Non-selected tabs originally before the group's first member; if the
        // group ends up empty we fall back to inserting at this position.
        let kept_before_group = group_first_index
            - selected_indices
                .iter()
                .filter(|&&i| i < group_first_index)
                .count();

        // Split tabs by index into the removed (selected) block and the rest.
        let (removed, mut rest): (Vec<_>, Vec<_>) =
            self.tabs
                .drain(..)
                .enumerate()
                .partition_map(|(index, tab)| {
                    if selected_set.contains(&index) {
                        Either::Left(tab)
                    } else {
                        Either::Right(tab)
                    }
                });
        // Anchor the removed block just after the group's remaining members;
        // if none remain, fall back to the pre-computed prefix position.
        let natural_insert_at = match rest.iter().rposition(|tab| tab.group_id == Some(group_id)) {
            Some(last) => last + 1,
            None => kept_before_group,
        };
        // The removed tabs are now unpinned (they left a possibly-pinned
        // group); they must land past every effectively pinned tab in
        // not just past the source group's remaining members.
        let insert_at = self.clamp_to_unpinned_region(&rest, natural_insert_at);
        rest.splice(insert_at..insert_at, removed);
        self.tabs = rest;

        self.clear_tab_multi_selection(ctx);
        self.restore_active_tab_index(active_pane_group_id);
        self.prune_empty_tab_group(group_id, ctx);

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    /// Items shown in the multi-tab right-click menu. Composition depends on
    /// the selection: "Create group from tabs" is always there; "Remove from
    /// group" only when the selection has an unambiguous group; "Move to
    /// group" only when there's a destination group worth offering.
    fn tab_selection_menu_items(&self) -> Vec<MenuItem<WorkspaceAction>> {
        let shared_group = self.selection_shared_group();
        let mut menu_items = vec![MenuItemFields::new("Create group from tabs")
            .with_on_select_action(WorkspaceAction::NewTabGroupFromSelectedTabs)
            .into_item()];

        // Only single-group selections have an unambiguous group to leave.
        if shared_group.is_some() {
            menu_items.push(
                MenuItemFields::new("Remove from group")
                    .with_on_select_action(WorkspaceAction::RemoveSelectedTabsFromGroup)
                    .into_item(),
            );
        }

        // Offer "Move to group" only when another group is available.
        let has_destination_group = self
            .tab_groups
            .keys()
            .any(|group_id| Some(*group_id) != shared_group);
        if has_destination_group {
            menu_items.push(MenuItemFields::new_submenu(MOVE_TO_GROUP_LABEL).into_item());
        }
        menu_items
    }

    /// Opens (or closes) the multi-tab right-click menu. Reuses the shared
    /// `tab_right_click_menu` view — the menu rendering pipeline doesn't need
    /// to know which item set is loaded, only which `show_*` flag is set.
    pub fn toggle_tab_selection_right_click_menu(
        &mut self,
        tab_index: usize,
        anchor: TabContextMenuAnchor,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.show_tab_selection_right_click_menu.is_some() {
            self.show_tab_selection_right_click_menu = None;
            self.hide_move_to_group_sidecar(ctx);
            ctx.notify();
            return;
        }

        let menu_items = self.tab_selection_menu_items();
        ctx.update_view(&self.tab_right_click_menu, |context_menu, view_ctx| {
            context_menu.set_items(menu_items, view_ctx);
        });
        self.show_tab_right_click_menu = None;
        self.show_tab_group_right_click_menu = None;
        self.hide_move_to_group_sidecar(ctx);
        self.show_tab_selection_right_click_menu = Some((tab_index, anchor));
        ctx.focus(&self.tab_right_click_menu);
        ctx.notify();
    }

    /// True when `tab` is positioned in the pinned region of the tab list —
    /// either because its own `pinned` flag is set (ungrouped pinned tab) or
    /// because it belongs to a pinned group.
    pub(super) fn is_tab_effectively_pinned(&self, tab: &TabData) -> bool {
        // Safety net, ensures no behavioral changes if feature flag
        // is off and some tabs have a pinned state saved.
        if !FeatureFlag::PinnedTabs.is_enabled() {
            return false;
        }
        tab.pinned
            || tab
                .group_id
                .is_some_and(|gid| self.tab_groups.get(&gid).is_some_and(|g| g.pinned))
    }

    /// Index where the unpinned region begins within `tabs`: the count of
    /// leading tabs that belong to the pinned region.
    pub(super) fn pinned_boundary_index(&self, tabs: &[TabData]) -> usize {
        tabs.iter()
            .take_while(|tab| self.is_tab_effectively_pinned(tab))
            .count()
    }

    /// Pushes `idx` past the leading effectively-pinned tabs in `tabs` if it
    /// falls inside that prefix.
    pub(super) fn clamp_to_unpinned_region(&self, tabs: &[TabData], idx: usize) -> usize {
        idx.max(self.pinned_boundary_index(tabs))
    }

    /// Returns the slot just past the last member of `group_id`, suitable as
    /// an insert/move target that keeps the group contiguous. `None` when the
    /// group has no members.
    pub(super) fn index_after_group(&self, group_id: TabGroupId) -> Option<usize> {
        group_member_indices(&self.tabs, group_id)
            .last()
            .map(|last| last + 1)
    }

    /// Pins the tab. Grouped tabs are extracted from their group first
    /// regardless of whether that group itself is pinned — tab pinning and
    /// group pinning are independent concepts.
    pub(super) fn pin_tab(&mut self, tab_index: usize, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::PinnedTabs.is_enabled() {
            return;
        }
        let Some(tab) = self.tabs.get(tab_index) else {
            log::debug!("pin_tab: tab_index {tab_index} out of bounds");
            return;
        };
        if tab.pinned {
            log::debug!("pin_tab: tab {tab_index} is already pinned");
            return;
        }
        let previous_group_id = tab.group_id;

        // Identify where this newly pinned tab should land (after the last pinned item).
        let target = self.pinned_boundary_index(&self.tabs);

        self.tabs[tab_index].group_id = None;
        self.tabs[tab_index].pinned = true;
        self.move_tab_to_index(tab_index, target, ctx);

        if let Some(prev) = previous_group_id {
            self.prune_empty_tab_group(prev, ctx);
        }

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    /// Unpins a pinned tab and moves it to the start of the unpinned region.
    pub(super) fn unpin_tab(&mut self, tab_index: usize, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::PinnedTabs.is_enabled() {
            return;
        }
        let Some(tab) = self.tabs.get(tab_index) else {
            log::debug!("unpin_tab: tab_index {tab_index} out of bounds");
            return;
        };
        if !tab.pinned {
            log::debug!("unpin_tab: tab {tab_index} is not pinned");
            return;
        }

        // This tab should land right after all pinned items.
        let target = self.pinned_boundary_index(&self.tabs);

        self.tabs[tab_index].pinned = false;
        self.move_tab_to_index(tab_index, target, ctx);

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    /// Pins the entire tab group: flips the group's `pinned` flag and moves
    /// its contiguous block of members to the end of the pinned region. We
    /// don't touch individual member `tab.pinned` flags because the block
    /// always travels as a unit, and we want to support pinning a tab even if
    /// it already belongs to a (pinned) group.
    pub(super) fn pin_tab_group(&mut self, group_id: TabGroupId, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::PinnedTabs.is_enabled() {
            return;
        }
        let Some(group) = self.tab_groups.get(&group_id) else {
            log::debug!("pin_tab_group: unknown group {group_id:?}");
            return;
        };
        if group.pinned {
            log::debug!("pin_tab_group: group {group_id:?} is already pinned");
            return;
        }

        let target = self.pinned_boundary_index(&self.tabs);
        if let Some(group) = self.tab_groups.get_mut(&group_id) {
            group.pinned = true;
        }
        self.move_group_block(group_id, target, ctx);

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    /// Unpins the entire tab group: clears the group's `pinned` flag and
    /// moves the group's block to the start of the unpinned region.
    pub(super) fn unpin_tab_group(&mut self, group_id: TabGroupId, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::PinnedTabs.is_enabled() {
            return;
        }
        let Some(group) = self.tab_groups.get(&group_id) else {
            log::debug!("unpin_tab_group: unknown group {group_id:?}");
            return;
        };
        if !group.pinned {
            log::debug!("unpin_tab_group: group {group_id:?} is not pinned");
            return;
        }

        let target = self.pinned_boundary_index(&self.tabs);
        if let Some(group) = self.tab_groups.get_mut(&group_id) {
            group.pinned = false;
        }
        self.move_group_block(group_id, target, ctx);

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    /// Builds the "Move to group" submenu. One builder serves both parent
    /// menus: `Some(tab_index)` for the single-tab pane menu, `None` for the
    /// multi-tab selection menu. Destination groups exclude the source's own
    /// group (no useful move) and follow panel order so the submenu visually
    /// matches what the user sees in the tabs sidebar.
    pub(super) fn build_move_to_group_sidecar_items(
        &self,
        tab_index: Option<usize>,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        // Exclude the source's current group (if any) — there's nowhere to
        // move it to. For a mixed selection (no shared group) every
        // destination stays available.
        let excluded_group = match tab_index {
            Some(idx) => self.tabs.get(idx).and_then(|tab| tab.group_id),
            None => self.selection_shared_group(),
        };

        let mut groups_with_first_index: Vec<(TabGroupId, usize)> = self
            .tab_groups
            .keys()
            .copied()
            .filter(|gid| Some(*gid) != excluded_group)
            .filter_map(|gid| {
                group_member_indices(&self.tabs, gid)
                    .next()
                    .map(|idx| (gid, idx))
            })
            .collect();
        groups_with_first_index.sort_by_key(|(_, idx)| *idx);

        groups_with_first_index
            .into_iter()
            .map(|(group_id, _)| {
                let label = self
                    .tab_groups
                    .get(&group_id)
                    .and_then(|g| g.name.clone())
                    .unwrap_or_else(|| "Untitled group".to_string());
                let action = match tab_index {
                    Some(tab_index) => WorkspaceAction::MoveTabToGroup {
                        tab_index,
                        group_id,
                    },
                    None => WorkspaceAction::MoveSelectedTabsToGroup { group_id },
                };
                MenuItemFields::new(label)
                    .with_on_select_action(action)
                    .into_item()
            })
            .collect()
    }
}
