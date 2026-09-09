use std::collections::HashMap;
use std::path::PathBuf;

use uuid::Uuid;
use warpui::{AppContext, Entity, SingletonEntity, ViewContext, WeakViewHandle, WindowId};

use crate::pane_group::{PaneGroup, PaneId};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DevContainerBuildKey {
    pub workspace_folder: PathBuf,
    pub config_file: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DevContainerBuildSurfaceStatus {
    Running,
    Failed,
}

#[derive(Clone)]
pub(crate) struct DevContainerBuildLocator {
    pub window_id: WindowId,
    pub pane_group: WeakViewHandle<PaneGroup>,
    pub pane_id: PaneId,
}

impl DevContainerBuildLocator {
    pub(crate) fn is_live(&self, ctx: &AppContext) -> bool {
        self.pane_group
            .upgrade(ctx)
            .is_some_and(|pane_group| pane_group.read(ctx, |group, _| group.has_pane(self.pane_id)))
    }

    pub(crate) fn focus_in_owner(&self, ctx: &mut ViewContext<PaneGroup>) {
        if self.window_id != ctx.window_id() {
            ctx.windows().show_window_and_focus_app(self.window_id);
        }
        if let Some(pane_group) = self.pane_group.upgrade(ctx) {
            pane_group.update(ctx, |group, ctx| {
                group.focus_pane_by_id(self.pane_id, ctx);
            });
        }
    }
}

pub(crate) struct DevContainerBuildRegistryEntry {
    pub operation_id: Uuid,
    pub locator: DevContainerBuildLocator,
    pub attempt_id: u64,
    pub surface: DevContainerBuildSurfaceStatus,
}

pub(crate) enum DevContainerBuildClaim {
    Existing {
        locator: DevContainerBuildLocator,
        _surface: DevContainerBuildSurfaceStatus,
        _operation_id: Uuid,
        _attempt_id: u64,
    },
    Claimed {
        _operation_id: Uuid,
    },
}

pub struct DevContainerBuildRegistry {
    entries: HashMap<DevContainerBuildKey, DevContainerBuildRegistryEntry>,
}

impl DevContainerBuildRegistry {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub(crate) fn claim(
        &mut self,
        key: DevContainerBuildKey,
        locator: DevContainerBuildLocator,
        operation_id: Uuid,
        ctx: &AppContext,
    ) -> DevContainerBuildClaim {
        if let Some(entry) = self.entries.get(&key) {
            if entry.locator.is_live(ctx) {
                return DevContainerBuildClaim::Existing {
                    locator: entry.locator.clone(),
                    _surface: entry.surface,
                    _operation_id: entry.operation_id,
                    _attempt_id: entry.attempt_id,
                };
            }
            self.entries.remove(&key);
        }
        self.entries.insert(
            key,
            DevContainerBuildRegistryEntry {
                operation_id,
                locator,
                attempt_id: 1,
                surface: DevContainerBuildSurfaceStatus::Running,
            },
        );
        DevContainerBuildClaim::Claimed {
            _operation_id: operation_id,
        }
    }

    pub(crate) fn get(
        &self,
        key: &DevContainerBuildKey,
    ) -> Option<&DevContainerBuildRegistryEntry> {
        self.entries.get(key)
    }

    pub(crate) fn set_attempt(&mut self, key: &DevContainerBuildKey, attempt_id: u64) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.attempt_id = attempt_id;
            entry.surface = DevContainerBuildSurfaceStatus::Running;
        }
    }

    pub(crate) fn mark_failed(&mut self, key: &DevContainerBuildKey) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.surface = DevContainerBuildSurfaceStatus::Failed;
        }
    }

    pub(crate) fn remove(&mut self, key: &DevContainerBuildKey) {
        self.entries.remove(key);
    }

    pub(crate) fn matches(
        &self,
        key: &DevContainerBuildKey,
        operation_id: Uuid,
        attempt_id: u64,
    ) -> bool {
        self.entries.get(key).is_some_and(|entry| {
            entry.operation_id == operation_id && entry.attempt_id == attempt_id
        })
    }
}

impl Default for DevContainerBuildRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Entity for DevContainerBuildRegistry {
    type Event = ();
}

impl SingletonEntity for DevContainerBuildRegistry {}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
