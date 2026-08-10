//! The reconciliation pass for automatic tab grouping.
//!
//! One entry point — [`Workspace::reconcile_tab_auto_group`] — decides and
//! applies the correct group for a single tab. It deliberately does *not* reuse
//! the manual grouping entry points in `tab_grouping.rs`: those clear pinned
//! flags, force-expand the destination group, and dispatch the inline rename
//! editor, none of which automation may do. It builds on the lower-level
//! primitives those paths share instead.

use std::collections::HashMap;
use std::path::PathBuf;

use warpui::{EntityId, ViewContext};

use super::{Workspace, group_has_single_member, group_member_indices};
use crate::workspace::project_key::{self, ProjectKey};
use crate::workspace::tab_group::{TabGroup, TabGroupId, auto_tab_grouping_available};

/// The resolver state automatic grouping keeps between reconciles.
///
/// In memory only: it merely has to survive between two directory changes, and
/// what has to survive a restart is persisted on the tab and the group instead.
#[derive(Default)]
pub(crate) struct AutoGroupingState {
    /// The project key each tab last resolved to, keyed by pane-group identity.
    ///
    /// Membership is tested against *this* key, never against the tab's current
    /// one: at the instant a tab's directory changes, a tab automation placed
    /// and a tab the user placed are state-identical — both sit in a group whose
    /// key no longer matches. Only the previously resolved key tells them apart.
    last_resolved_keys: HashMap<EntityId, ProjectKey>,
}

impl AutoGroupingState {
    fn last_resolved_key(&self, pane_group_id: EntityId) -> Option<&ProjectKey> {
        self.last_resolved_keys.get(&pane_group_id)
    }

    fn record_resolved_key(&mut self, pane_group_id: EntityId, key: ProjectKey) {
        self.last_resolved_keys.insert(pane_group_id, key);
    }
}

impl Workspace {
    /// Reconciles one tab's group membership against the project key its
    /// directory currently resolves to.
    ///
    /// Identified by pane group rather than by tab index on purpose: repository
    /// detection answers an arbitrary number of frames after the directory
    /// changed, so an index captured before the answer arrived can address a
    /// different tab by now. The identity is resolved to an index immediately
    /// before any mutation, and a vanished identity is a no-op.
    ///
    /// `resolved_key` is `None` whenever project identity could not be
    /// established — no directory, or detection still pending. That is not a
    /// manual act, so the tab is left exactly where it is and stays queued for
    /// placement.
    ///
    /// Callers own resolving the key (and therefore which pane anchors it).
    pub fn reconcile_tab_auto_group(
        &mut self,
        pane_group_id: EntityId,
        resolved_key: Option<ProjectKey>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !auto_tab_grouping_available() {
            return;
        }
        let Some(tab_index) = self.tab_index_for_pane_group(pane_group_id) else {
            return;
        };

        // A pinned tab is never grouped and never loses its pin. Skipping it
        // outright is what keeps automation clear of the manual join path,
        // whose only way into a group clears the pin.
        if self.tabs[tab_index].pinned {
            return;
        }

        // Identity is unknown: leave the tab where it is, and leave the marker
        // set so it is still placed once its key does resolve.
        let Some(key) = resolved_key else {
            return;
        };
        let key_storage = key.to_storage_string();

        let current_group_id = self.tabs[tab_index].group_id;
        let current_group_key = current_group_id.and_then(|group_id| {
            self.tab_groups
                .get(&group_id)
                .and_then(|group| group.project_key.clone())
        });

        // Already in the group for its current key. Nothing to move, but the
        // tab is demonstrably under automation again — this is also how a tab
        // the user detached re-attaches by being put back where it belongs.
        if current_group_key.as_deref() == Some(key_storage.as_str()) {
            let was_awaiting_placement =
                std::mem::replace(&mut self.tabs[tab_index].placed_by_automation, false);
            self.auto_grouping_state
                .record_resolved_key(pane_group_id, key);
            if was_awaiting_placement {
                ctx.dispatch_global_action("workspace:save_app", ());
                ctx.notify();
            }
            return;
        }

        // Under automation? Either the group the tab sits in still carries the
        // key the tab last resolved to — which is exactly the state a tracked
        // tab is in the moment its directory changes — or the tab has never
        // been placed at all. Any other mismatch is a placement the user made,
        // and a manual act is never undone.
        let previous_key_storage = self
            .auto_grouping_state
            .last_resolved_key(pane_group_id)
            .map(ProjectKey::to_storage_string);
        let sits_in_previous_keys_group = match (&current_group_key, &previous_key_storage) {
            (Some(group_key), Some(previous_key)) => group_key == previous_key,
            _ => false,
        };
        let under_automation =
            sits_in_previous_keys_group || self.tabs[tab_index].placed_by_automation;

        // Record before any early return: the next reconcile has to compare
        // against the key seen now, including for a tab left alone here, or a
        // detached tab dragged back into the group it originally came from
        // would read as tracked again.
        self.auto_grouping_state
            .record_resolved_key(pane_group_id, key.clone());

        if !under_automation {
            return;
        }

        let active_pane_group_id = self
            .tabs
            .get(self.active_tab_index)
            .map(|tab| tab.pane_group.id());

        if let Some(destination_group_id) = self.group_for_project_key(&key_storage) {
            self.join_keyed_group(tab_index, destination_group_id, ctx);
        } else if let Some(group_id) = current_group_id
            && self
                .tab_groups
                .get(&group_id)
                .is_some_and(|group| group.project_key.is_some())
            && group_has_single_member(&self.tabs, group_id)
        {
            // Re-key the group in place instead of destroying and recreating
            // it, so a group holding a single tab does not flicker on every
            // `cd`. The tab does not move.
            self.rekey_group_in_place(group_id, &key);
        } else {
            self.create_keyed_group_for_tab(tab_index, &key, ctx);
        }

        // A reorder must not go through the normal activation path, so the
        // active tab is re-seated by identity.
        self.restore_active_tab_index(active_pane_group_id);

        if let Some(previous_group_id) = current_group_id {
            self.prune_empty_tab_group(previous_group_id, ctx);
        }

        // The tab moved, so its index may have changed: clear the marker by
        // identity rather than by the index captured above.
        if let Some(tab_index) = self.tab_index_for_pane_group(pane_group_id) {
            self.tabs[tab_index].placed_by_automation = false;
        }

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    /// Appends the tab to the end of `group_id`'s contiguous run.
    ///
    /// Unlike the manual join path this leaves the destination group's collapse
    /// state alone: collapsing is a manual act automation must not undo.
    fn join_keyed_group(
        &mut self,
        tab_index: usize,
        group_id: TabGroupId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Computed before membership changes, so the slot is past the group's
        // existing members rather than past the tab we are about to add.
        let target = self.index_after_group(group_id).unwrap_or(self.tabs.len());
        self.assign_tab_to_group(tab_index, Some(group_id), ctx);
        self.move_tab_to_index(tab_index, target, ctx);
    }

    /// Points an existing group at `key`, keeping a name the user set.
    ///
    /// Name provenance is derived rather than stored: a name that differs from
    /// what the group's own key would have produced was typed by the user and
    /// survives the re-key; one that matches was derived and is replaced.
    fn rekey_group_in_place(&mut self, group_id: TabGroupId, key: &ProjectKey) {
        let Some(group) = self.tab_groups.get(&group_id) else {
            return;
        };
        let previous_name = group.name.clone();
        let previously_derived_name = group
            .project_key
            .clone()
            .map(|stored| ProjectKey::from_path(PathBuf::from(stored)))
            .map(|previous_key| self.derived_group_name(&previous_key));
        let name_was_derived = match (&previous_name, &previously_derived_name) {
            // An unnamed group has nothing of the user's to protect.
            (None, _) => true,
            (Some(name), Some(derived)) => name == derived,
            (Some(_), None) => false,
        };

        if let Some(group) = self.tab_groups.get_mut(&group_id) {
            group.project_key = Some(key.to_storage_string());
        }
        if name_was_derived {
            // Derived after the re-key so the name is qualified against the
            // window's keys as they stand now.
            let name = self.derived_group_name(key);
            if let Some(group) = self.tab_groups.get_mut(&group_id) {
                group.name = Some(name);
            }
        }
    }

    /// Creates a group carrying `key` and moves the tab into it.
    ///
    /// Automation's own creation path: it never dispatches the deferred rename
    /// action both manual entry points end with, which would open the inline
    /// rename editor and steal focus.
    fn create_keyed_group_for_tab(
        &mut self,
        tab_index: usize,
        key: &ProjectKey,
        ctx: &mut ViewContext<Self>,
    ) {
        let previous_group_id = self.tabs[tab_index].group_id;
        // Anchor the new group at the tab's own slot, or — when the tab is
        // leaving a group — just past that group's last remaining member, so
        // the group it leaves stays contiguous. Then clamp past the pinned
        // region: the new group is unpinned and must not land inside it.
        let natural_target = previous_group_id
            .and_then(|group_id| self.index_after_group(group_id))
            .unwrap_or(tab_index);
        let target = self.clamp_to_unpinned_region(&self.tabs, natural_target);

        let mut group = TabGroup::new();
        let group_id = group.id;
        group.project_key = Some(key.to_storage_string());
        self.tab_groups.insert(group_id, group);
        // Named after insertion so the name is qualified against every key in
        // the window, this one included.
        let name = self.derived_group_name(key);
        if let Some(group) = self.tab_groups.get_mut(&group_id) {
            group.name = Some(name);
        }

        self.assign_tab_to_group(tab_index, Some(group_id), ctx);
        self.move_tab_to_index(tab_index, target, ctx);
    }

    /// The group carrying `key_storage`, if there is one. Ties break on the
    /// group's first member and then on its id, so the choice never depends on
    /// hash-map iteration order.
    fn group_for_project_key(&self, key_storage: &str) -> Option<TabGroupId> {
        self.tab_groups
            .values()
            .filter(|group| group.project_key.as_deref() == Some(key_storage))
            .min_by_key(|group| {
                let first_member = group_member_indices(&self.tabs, group.id)
                    .next()
                    .unwrap_or(usize::MAX);
                (first_member, group.id.0)
            })
            .map(|group| group.id)
    }

    /// The name automatic grouping would give a group keyed to `key`.
    fn derived_group_name(&self, key: &ProjectKey) -> String {
        let others = self.project_keys_in_window();
        project_key::display_name(key, others.iter())
    }

    /// Every project key in use by a group in this window.
    fn project_keys_in_window(&self) -> Vec<ProjectKey> {
        self.tab_groups
            .values()
            .filter_map(|group| group.project_key.as_ref())
            .map(|stored| ProjectKey::from_path(PathBuf::from(stored)))
            .collect()
    }

    fn tab_index_for_pane_group(&self, pane_group_id: EntityId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.pane_group.id() == pane_group_id)
    }
}

#[cfg(test)]
#[path = "auto_grouping_tests.rs"]
mod tests;
