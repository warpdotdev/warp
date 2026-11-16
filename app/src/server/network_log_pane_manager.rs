//! Tracks open [`NetworkLogPane`]s across windows so that we show at most one
//! per window and can focus the existing one when reopened.
//!
//! Mirrors the pattern used by [`crate::ai::execution_profiles::editor::manager::ExecutionProfileEditorManager`].
use std::collections::HashMap;

use warpui::{Entity, SingletonEntity, WindowId};

use crate::workspace::PaneViewLocator;

/// Singleton that maintains a map of `WindowId -> PaneViewLocator` for any open
/// network log panes.
#[derive(Default)]
pub struct NetworkLogPaneManager {
    panes: HashMap<WindowId, PaneViewLocator>,
}

impl NetworkLogPaneManager {
    pub fn find_pane(&self, window_id: WindowId) -> Option<PaneViewLocator> {
        self.panes.get(&window_id).copied()
    }

    pub fn register_pane(&mut self, window_id: WindowId, locator: PaneViewLocator) {
        self.panes.insert(window_id, locator);
    }

    pub fn deregister_pane(&mut self, window_id: &WindowId) {
        self.panes.remove(window_id);
    }
}

impl Entity for NetworkLogPaneManager {
    type Event = ();
}

impl SingletonEntity for NetworkLogPaneManager {}
