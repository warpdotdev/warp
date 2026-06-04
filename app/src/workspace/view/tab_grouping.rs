use std::collections::HashSet;

use warp_core::features::FeatureFlag;
use warpui::ViewContext;

use super::Workspace;
use crate::workspace::tab_group::TabGroupId;
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
        self.tab_groups
            .iter_mut()
            .filter(|(group_id, _)| crossed_group_ids.contains(group_id))
            .for_each(|(_, group)| group.collapsed = false);

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
}
