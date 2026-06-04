use std::collections::HashSet;
use warp_core::features::FeatureFlag;
use warpui::ViewContext;

use super::Workspace;
use crate::workspace::tab_group::TabGroupId;
use crate::workspace::util::PaneViewLocator;

// TODO(johnturcoo) move tab grouping helpers here from workspace/view.rs.
impl Workspace {
    /// Clears the range selection.
    pub(super) fn clear_tab_range_selection(&mut self, ctx: &mut ViewContext<Self>) {
        let mut changed = false;
        for tab in &mut self.tabs {
            if tab.in_range_selected {
                tab.in_range_selected = false;
                changed = true;
            }
        }
        if changed {
            ctx.notify();
        }
    }

    /// Selects the inclusive range between `anchor_index` and `clicked_index`,
    /// expanding any collapsed groups the range crosses.
    fn set_tab_range_selection(
        &mut self,
        anchor_index: usize,
        clicked_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        // Determine the bounds for our range selection.
        let lo = anchor_index.min(clicked_index);
        let hi = anchor_index.max(clicked_index);

        // Identify groups in the selection range.
        let crossed_group_ids: HashSet<TabGroupId> = self
            .tabs
            .get(lo..=hi)
            .into_iter()
            .flatten()
            .filter_map(|tab| tab.group_id)
            .collect();
        let mut expanded_any = false;

        // Expand any groups within the selected range, so user can see what they are selecting.
        for group_id in crossed_group_ids {
            if let Some(group) = self.tab_groups.get_mut(&group_id) {
                if group.collapsed {
                    group.collapsed = false;
                    expanded_any = true;
                }
            }
        }

        // Update flag for all tabs that are in the selected range.
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            tab.in_range_selected = index >= lo && index <= hi;
        }
        if expanded_any {
            ctx.dispatch_global_action("workspace:save_app", ());
        }
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
        let Some(clicked_index) = self
            .tabs
            .iter()
            .position(|tab| tab.pane_group.id() == locator.pane_group_id)
        else {
            return;
        };
        self.set_tab_range_selection(self.active_tab_index, clicked_index, ctx);
    }
}
