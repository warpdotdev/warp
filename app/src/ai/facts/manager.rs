use std::collections::HashMap;

use warpui::{Entity, EntityId, ModelContext, SingletonEntity, ViewHandle, WindowId};

use crate::PaneViewLocator;
use crate::ai::facts::AIFactView;
use crate::pane_group::{AIFactPane, PaneContent};

/// Singleton model to manage state of AI fact panes across multiple windows
/// (where only one AI fact pane can exist per window). Specifically:
/// - Maintains AI fact view handles to preserve state when panes are hidden
/// - Tracks currently open AI fact panes and their location
#[derive(Default)]
pub struct AIFactManager {
    panes: HashMap<WindowId, AIFactPaneData>,
}

struct AIFactPaneData {
    locator: Option<PaneViewLocator>,
    view: ViewHandle<AIFactView>,
}

impl AIFactManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ai_fact_view(&self, window_id: WindowId) -> ViewHandle<AIFactView> {
        self.panes
            .get(&window_id)
            .expect("Window should have corresponding AI fact view")
            .view
            .clone()
    }

    pub fn register_view(&mut self, window_id: WindowId, view: ViewHandle<AIFactView>) {
        if let Some(data) = self.panes.get_mut(&window_id) {
            data.view = view;
        } else {
            self.panes.insert(
                window_id,
                AIFactPaneData {
                    locator: None,
                    view,
                },
            );
        }
    }

    pub fn find_pane(&self, window_id: WindowId) -> Option<PaneViewLocator> {
        self.panes.get(&window_id).and_then(|data| data.locator)
    }

    pub fn register_pane(
        &mut self,
        pane: &AIFactPane,
        pane_group_id: EntityId,
        window_id: WindowId,
        _ctx: &mut ModelContext<Self>,
    ) {
        if let Some(data) = self.panes.get_mut(&window_id) {
            data.locator = Some(PaneViewLocator {
                pane_group_id,
                pane_id: pane.id(),
            });
        } else {
            log::warn!("AI fact view should already exist for AI fact pane");
        }
    }

    pub fn deregister_pane(&mut self, window_id: &WindowId, _ctx: &mut ModelContext<Self>) {
        if let Some(data) = self.panes.get_mut(window_id) {
            data.locator = None;
        }
    }
}

impl Entity for AIFactManager {
    type Event = ();
}

impl SingletonEntity for AIFactManager {}
