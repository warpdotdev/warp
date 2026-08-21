use std::collections::HashMap;

use warpui::{Entity, EntityId, ModelContext, SingletonEntity, ViewHandle, WindowId};

use super::SettingsView;
use crate::PaneViewLocator;
use crate::pane_group::{PaneContent, PaneId, SettingsPane};

struct SettingsPaneData {
    locator: Option<PaneViewLocator>,
    settings_view: ViewHandle<SettingsView>,
}

/// Singleton model to manage state of settings panes across multiple windows
/// (where only one settings pane can exist per window). Specifically:
/// - Maintains settings view handles to preserve state when panes are hidden
/// - Tracks currently open settings panes and their location
#[derive(Default)]
pub struct SettingsPaneManager {
    panes: HashMap<WindowId, SettingsPaneData>,
}

impl SettingsPaneManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn settings_view(&self, window_id: WindowId) -> ViewHandle<SettingsView> {
        self.panes
            .get(&window_id)
            .expect("Window should have corresponding settings view")
            .settings_view
            .clone()
    }

    pub fn register_view(&mut self, window_id: WindowId, view: ViewHandle<SettingsView>) {
        if let Some(data) = self.panes.get_mut(&window_id) {
            data.settings_view = view;
        } else {
            self.panes.insert(
                window_id,
                SettingsPaneData {
                    locator: None,
                    settings_view: view,
                },
            );
        }
    }

    pub fn find_pane(&self, window_id: WindowId) -> Option<PaneViewLocator> {
        self.panes.get(&window_id).and_then(|data| data.locator)
    }

    pub fn register_pane(
        &mut self,
        pane: &SettingsPane,
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
            log::warn!("Settings view should already exist for settings pane");
        }
    }

    /// Registers `pane` as transferred into `window_id`, preserving the
    /// invariant that at most one Settings pane is tracked per window. If
    /// `window_id` already has a *different* live pane registered, the
    /// existing registration is left untouched and its locator is returned
    /// so the caller can reconcile the collision (the transferred pane must
    /// be discarded and the existing one kept). `None` means there was no
    /// collision -- the slot was empty, or already pointed at this exact
    /// pane -- and the transferred pane is now the registered one.
    pub fn register_transferred_pane(
        &mut self,
        pane: &SettingsPane,
        pane_group_id: EntityId,
        window_id: WindowId,
        _ctx: &mut ModelContext<Self>,
    ) -> Option<PaneViewLocator> {
        let incoming = PaneViewLocator {
            pane_group_id,
            pane_id: pane.id(),
        };
        let Some(data) = self.panes.get_mut(&window_id) else {
            debug_assert!(
                false,
                "SettingsPaneManager::register_transferred_pane: destination window {window_id:?} has no tracked SettingsPaneData; every window is expected to call `register_view` before any pane can transfer into it. Silently dropping this pane's registration would corrupt the one-pane-per-window invariant."
            );
            log::warn!("Settings view should already exist for settings pane");
            return None;
        };
        match data.locator {
            Some(existing) if existing != incoming => Some(existing),
            _ => {
                data.locator = Some(incoming);
                None
            }
        }
    }

    pub fn deregister_pane(
        &mut self,
        window_id: &WindowId,
        pane_group_id: EntityId,
        pane_id: PaneId,
        _ctx: &mut ModelContext<Self>,
    ) {
        if let Some(data) = self.panes.get_mut(window_id) {
            let locator = PaneViewLocator {
                pane_group_id,
                pane_id,
            };
            if data.locator == Some(locator) {
                data.locator = None;
            }
        }
    }
}

impl Entity for SettingsPaneManager {
    type Event = ();
}

/// Mark SettingsPaneManager as global application state.
impl SingletonEntity for SettingsPaneManager {}
