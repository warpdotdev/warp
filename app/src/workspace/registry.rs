use std::collections::HashMap;

use warpui::{AppContext, Entity, SingletonEntity, WeakViewHandle, WindowId};

use super::Workspace;

/// A registry that tracks all workspace views by their window ID.
///
/// This provides O(1) lookup of workspaces instead of the O(n) linear scan
/// that `views_of_type::<Workspace>` performs.
pub struct WorkspaceRegistry {
    workspaces: HashMap<WindowId, WeakViewHandle<Workspace>>,
}

impl Default for WorkspaceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceRegistry {
    pub fn new() -> Self {
        Self {
            workspaces: HashMap::new(),
        }
    }

    /// Registers a workspace for the given window.
    pub fn register(&mut self, window_id: WindowId, workspace: WeakViewHandle<Workspace>) {
        self.workspaces.insert(window_id, workspace);
    }

    /// Unregisters the workspace for the given window.
    pub fn unregister(&mut self, window_id: WindowId) {
        self.workspaces.remove(&window_id);
    }

    /// Returns the workspace for the given window, if it is still alive.
    pub fn get(
        &self,
        window_id: WindowId,
        app: &AppContext,
    ) -> Option<warpui::ViewHandle<Workspace>> {
        self.workspaces.get(&window_id)?.upgrade(app)
    }

    /// Returns all registered workspaces that are still alive.
    /// The returned vector contains tuples of (WindowId, ViewHandle<Workspace>).
    pub fn all_workspaces(
        &self,
        app: &AppContext,
    ) -> Vec<(WindowId, warpui::ViewHandle<Workspace>)> {
        self.workspaces
            .iter()
            .filter_map(|(window_id, weak_handle)| {
                weak_handle.upgrade(app).map(|handle| (*window_id, handle))
            })
            .collect()
    }
}

impl Entity for WorkspaceRegistry {
    type Event = ();
}

impl SingletonEntity for WorkspaceRegistry {}
