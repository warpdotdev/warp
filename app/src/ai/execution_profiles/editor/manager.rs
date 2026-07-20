use std::collections::HashMap;

use warpui::{Entity, EntityId, ModelContext, SingletonEntity, WindowId};

use crate::PaneViewLocator;
use crate::ai::execution_profiles::profiles::ClientProfileId;
use crate::pane_group::{ExecutionProfileEditorPane, PaneContent};

/// Manages execution profile editor panes across different windows and profiles.
///
/// This manager tracks which execution profile editor panes are active in each window,
/// allowing the application to locate and interact with these panes when needed.
/// It maintains a mapping from each window ID to a map from profile IDs to pane data,
/// including the locator information needed to find and reference specific panes within their pane groups.
#[derive(Default)]
pub struct ExecutionProfileEditorManager {
    panes: HashMap<WindowId, HashMap<ClientProfileId, ExecutionProfileEditorPaneData>>,
}

#[derive(Clone, Copy)]
struct ExecutionProfileEditorPaneData {
    locator: PaneViewLocator,
}

impl ExecutionProfileEditorManager {
    pub fn find_pane(
        &self,
        window_id: WindowId,
        profile_id: ClientProfileId,
    ) -> Option<PaneViewLocator> {
        self.panes
            .get(&window_id)
            .and_then(|m| m.get(&profile_id))
            .map(|d| d.locator)
    }

    pub fn register_pane(
        &mut self,
        pane: &ExecutionProfileEditorPane,
        pane_group_id: EntityId,
        window_id: WindowId,
        profile_id: ClientProfileId,
        _ctx: &mut ModelContext<Self>,
    ) {
        let locator = PaneViewLocator {
            pane_group_id,
            pane_id: pane.id(),
        };
        self.panes
            .entry(window_id)
            .or_default()
            .insert(profile_id, ExecutionProfileEditorPaneData { locator });
    }

    pub fn deregister_pane(&mut self, window_id: &WindowId, profile_id: &ClientProfileId) {
        if let Some(map) = self.panes.get_mut(window_id) {
            map.remove(profile_id);
            if map.is_empty() {
                self.panes.remove(window_id);
            }
        }
    }
}

impl Entity for ExecutionProfileEditorManager {
    type Event = ();
}

impl SingletonEntity for ExecutionProfileEditorManager {}
