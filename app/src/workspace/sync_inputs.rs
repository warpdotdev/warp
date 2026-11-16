use std::collections::{HashMap, HashSet};

use warpui::keymap::EditableBinding;
use warpui::{AppContext, Entity, EntityId, SingletonEntity, WindowId};

use super::WorkspaceAction;
use crate::util::bindings::{BindingGroup, CustomAction};

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_editable_bindings(vec![
        EditableBinding::new(
            "workspace:disable_terminal_input_syncing",
            "Stop Synchronizing Any Panes",
            WorkspaceAction::DisableTerminalInputSync,
        )
        .with_context_predicate(id!("Workspace"))
        .with_key_binding("alt-cmd-shift-I")
        .with_group(BindingGroup::Settings.as_str())
        .with_custom_action(CustomAction::DisableSyncTerminalInputs),
        EditableBinding::new(
            "workspace:toggle_sync_terminal_inputs_in_tab",
            "Toggle Synchronizing All Panes in Current Tab",
            WorkspaceAction::ToggleSyncTerminalInputsInTab,
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(BindingGroup::Settings.as_str())
        .with_custom_action(CustomAction::ToggleSyncTerminalInputsInCurrentTab),
        EditableBinding::new(
            "workspace:toggle_sync_all_terminal_inputs_in_all_tabs",
            "Toggle Synchronizing All Panes in All Tabs",
            WorkspaceAction::ToggleSyncAllTerminalInputsInAllTabs,
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(BindingGroup::Settings.as_str())
        .with_custom_action(CustomAction::ToggleSyncAllTerminalInputsInAllTabs),
    ]);
}

#[derive(Debug, PartialEq)]
enum SyncedPanes {
    All,
    AllPanesInPaneGroups { pane_group_ids: HashSet<EntityId> },
}

/// Stores state for syncing inputs across terminals.
/// Note: we sync input editors with themselves and
/// alt-screen/long-running commands with themselves
pub struct SyncedInputState {
    sync_state_by_window: HashMap<WindowId, Option<SyncedPanes>>,
}

impl Entity for SyncedInputState {
    type Event = ();
}

impl SingletonEntity for SyncedInputState {}

impl Default for SyncedInputState {
    fn default() -> Self {
        SyncedInputState::new()
    }
}

impl SyncedInputState {
    pub fn new() -> Self {
        Self {
            sync_state_by_window: HashMap::new(),
        }
    }

    pub fn mock() -> Self {
        Self::new()
    }

    pub fn toggle_sync_all_terminal_inputs_in_all_tabs(&mut self, window_id: WindowId) {
        let new_sync_state = match self.sync_state_by_window.get(&window_id).unwrap_or(&None) {
            Some(SyncedPanes::All) => None,
            _ => Some(SyncedPanes::All),
        };

        self.sync_state_by_window.insert(window_id, new_sync_state);
    }

    pub fn toggle_sync_terminal_inputs_in_tab(
        &mut self,
        tab_id: EntityId,
        all_tab_ids: impl Iterator<Item = EntityId>,
        pane_group_count: usize,
        window_id: WindowId,
    ) {
        let new_state = match self.sync_state_by_window.get(&window_id).unwrap_or(&None) {
            None => {
                let mut synced_tabs = HashSet::new();
                synced_tabs.insert(tab_id);

                Some(SyncedPanes::AllPanesInPaneGroups {
                    pane_group_ids: synced_tabs,
                })
            }
            Some(SyncedPanes::All) => {
                let mut synced_tabs = HashSet::from_iter(all_tab_ids);
                synced_tabs.remove(&tab_id);

                Self::normalized_synced_panes(synced_tabs, pane_group_count)
            }
            Some(SyncedPanes::AllPanesInPaneGroups {
                pane_group_ids: tab_ids,
            }) => {
                let mut synced_tabs = tab_ids.clone();
                if synced_tabs.contains(&tab_id) {
                    // Tab is already synced so toggle should un-sync it.
                    synced_tabs.remove(&tab_id);
                } else {
                    // Tab wasn't already synced so toggle should sync it.
                    synced_tabs.insert(tab_id);
                }

                Self::normalized_synced_panes(synced_tabs, pane_group_count)
            }
        };

        self.sync_state_by_window.insert(window_id, new_state);
    }

    /// Given a set of `synced_pane_group_ids` and the total count of pane groups in a window, return the normalized SyncedPane variant to reduce ambiguity. For example, if `synced_pane_group_ids` is an empty HashSet, the normalized representation should be None.
    fn normalized_synced_panes(
        synced_pane_group_ids: HashSet<EntityId>,
        pane_group_count: usize,
    ) -> Option<SyncedPanes> {
        match synced_pane_group_ids.len() {
            0 => None,
            i if i == pane_group_count => Some(SyncedPanes::All),
            _ => Some(SyncedPanes::AllPanesInPaneGroups {
                pane_group_ids: synced_pane_group_ids,
            }),
        }
    }

    pub fn disable_sync_terminal_inputs(&mut self, window_id: WindowId) {
        self.sync_state_by_window.insert(window_id, None);
    }

    fn get_state(&self, window_id: WindowId) -> Option<&SyncedPanes> {
        self.sync_state_by_window
            .get(&window_id)
            .and_then(|state| state.as_ref())
    }

    pub fn is_syncing_any_inputs(&self, window_id: WindowId) -> bool {
        self.get_state(window_id).is_some()
    }

    pub fn is_syncing_all_inputs(&self, window_id: WindowId) -> bool {
        matches!(self.get_state(window_id), Some(SyncedPanes::All))
    }

    /// Returns true if sync mode is all panes in a set of pane group ids and
    /// the specified pane group id is in that set.
    /// Returns false otherwise -- notably, even when all panes are synced.
    /// Useful when we need to know the exact sync state, not just sync.
    pub fn is_syncing_all_panes_in_pane_group(
        &self,
        window_id: WindowId,
        pane_group_id: EntityId,
    ) -> bool {
        match self.sync_state_by_window.get(&window_id).unwrap_or(&None) {
            Some(SyncedPanes::AllPanesInPaneGroups {
                pane_group_ids: tab_ids,
            }) => tab_ids.contains(&pane_group_id),
            _ => false,
        }
    }
    /// Returns true if we're in any state that should sync this pane group.
    pub fn should_sync_this_pane_group(
        &self,
        pane_group_id: EntityId,
        window_id: WindowId,
    ) -> bool {
        match self.sync_state_by_window.get(&window_id).unwrap_or(&None) {
            Some(SyncedPanes::All) => true,
            Some(SyncedPanes::AllPanesInPaneGroups {
                pane_group_ids: tab_ids,
            }) => tab_ids.contains(&pane_group_id),
            None => false,
        }
    }
}
