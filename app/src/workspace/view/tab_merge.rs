use warp_core::features::FeatureFlag;
use warpui::{AppContext, ViewContext};

use super::Workspace;
use crate::tab::MergeDestination;

impl Workspace {
    /// Every tab other than the source, in tab order, for the "Merge into tab" menu entries.
    pub(super) fn merge_into_tab_destinations(
        &self,
        source_tab_index: usize,
        ctx: &AppContext,
    ) -> Vec<MergeDestination> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != source_tab_index)
            .map(|(index, tab)| MergeDestination {
                index,
                title: tab.pane_group.as_ref(ctx).display_title(ctx),
            })
            .collect()
    }

    /// Moves every pane of the source tab into the destination tab as splits, keeping their
    /// sessions alive, then closes the emptied source tab and activates the destination.
    ///
    /// No-ops when the source tab holds anything other than visible terminal panes, so a tab is
    /// never half-merged.
    pub(super) fn merge_tab_into_tab(
        &mut self,
        source_tab_index: usize,
        destination_tab_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::MergeTabIntoTab.is_enabled() || source_tab_index == destination_tab_index {
            return;
        }
        let (Some(source_group), Some(destination_group)) = (
            self.get_pane_group_view(source_tab_index).cloned(),
            self.get_pane_group_view(destination_tab_index).cloned(),
        ) else {
            log::debug!(
                "merge_tab_into_tab: tab index out of bounds ({source_tab_index} -> {destination_tab_index})"
            );
            return;
        };
        if !source_group.as_ref(ctx).can_merge_into_another_tab() {
            return;
        }

        let panes = source_group.update(ctx, |group, ctx| group.take_panes_for_merge(ctx));
        destination_group.update(ctx, |group, ctx| group.add_merged_panes(panes, ctx));

        // Activating the destination before closing the source keeps the source out of the
        // activation history and stops the close from activating a neighbor. Closing here,
        // rather than waiting for the source group's `Exited` event, settles the final tab
        // state before this action finishes; that event then finds no matching tab and is a
        // no-op.
        self.activate_tab_by_pane_group_id(destination_group.id(), ctx);
        if let Some(source_tab_index) = self
            .tabs
            .iter()
            .position(|tab| tab.pane_group.id() == source_group.id())
        {
            self.close_tab(source_tab_index, true, false, ctx);
        }
    }
}
