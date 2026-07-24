//! Shared projection of cloud environments for GUI and TUI consumers.

use settings::Setting as _;
use warp_errors::report_if_error;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity as _};

use super::CloudAmbientAgentEnvironment;
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::cloud_object::CloudObjectLookup as _;
use crate::cloud_object::model::generic_string_model::StringModel as _;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::SyncId;

/// Environment identity and display data consumed by frontend selectors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudEnvironment {
    pub id: SyncId,
    pub name: String,
}

/// Emitted when the projected environment catalog changes.
#[derive(Clone, Copy, Debug)]
pub struct CloudEnvironmentCatalogEvent;

/// Canonical, recency-ordered cloud-environment projection shared by frontends.
pub struct CloudEnvironmentCatalog {
    environments: Vec<CloudEnvironment>,
}

impl CloudEnvironmentCatalog {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&CloudModel::handle(ctx), |catalog, _, event, ctx| {
            match event {
                // `CloudModel::create_object` emits before inserting. Defer the
                // projection lookup until the source update and event flush finish.
                CloudModelEvent::ObjectCreated { .. } => {
                    ctx.spawn(async {}, |catalog, (), ctx| catalog.refresh(ctx));
                }
                CloudModelEvent::InitialLoadCompleted
                | CloudModelEvent::ObjectMoved { .. }
                | CloudModelEvent::ObjectUpdated { .. }
                | CloudModelEvent::ObjectTrashed { .. }
                | CloudModelEvent::ObjectUntrashed { .. }
                | CloudModelEvent::NotebookEditorChangedFromServer { .. }
                | CloudModelEvent::ObjectDeleted { .. }
                | CloudModelEvent::ObjectPermissionsUpdated { .. }
                | CloudModelEvent::ObjectForceExpanded { .. }
                | CloudModelEvent::ObjectSynced { .. } => catalog.refresh(ctx),
            }
        });
        Self {
            environments: Self::current_environments(ctx),
        }
    }

    /// Current environments ordered by most-recent use, then display name.
    pub fn environments(&self) -> &[CloudEnvironment] {
        &self.environments
    }

    /// Returns the projected environment with `id`.
    pub fn environment(&self, id: SyncId) -> Option<&CloudEnvironment> {
        self.environments
            .iter()
            .find(|environment| environment.id == id)
    }

    /// Returns the saved environment when it still exists, otherwise the
    /// most-recent environment.
    pub fn default_environment_id(&self, ctx: &AppContext) -> Option<SyncId> {
        let saved = *CloudAgentSettings::as_ref(ctx)
            .last_selected_environment_id
            .value();
        saved
            .filter(|id| self.environment(*id).is_some())
            .or_else(|| self.environments.first().map(|environment| environment.id))
    }

    /// Persists a valid environment selection for future default resolution.
    pub fn persist_selection(&self, environment_id: SyncId, ctx: &mut ModelContext<Self>) {
        if self.environment(environment_id).is_none() {
            return;
        }
        CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(
                settings
                    .last_selected_environment_id
                    .set_value(Some(environment_id), ctx)
            );
        });
    }

    /// Requests an out-of-band refresh of cloud objects from the server.
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub fn refresh_from_server(&self, ctx: &mut ModelContext<Self>) {
        UpdateManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.refresh_updated_objects(ctx);
        });
    }

    fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        let environments = Self::current_environments(ctx);
        if environments != self.environments {
            self.environments = environments;
            ctx.emit(CloudEnvironmentCatalogEvent);
            ctx.notify();
        }
    }

    fn current_environments(ctx: &AppContext) -> Vec<CloudEnvironment> {
        let mut environments = CloudAmbientAgentEnvironment::get_all(ctx);
        sort_environments_by_recency(&mut environments);
        environments
            .into_iter()
            .map(|environment| CloudEnvironment {
                id: environment.id,
                name: environment.model().string_model.display_name(),
            })
            .collect()
    }
}

impl Entity for CloudEnvironmentCatalog {
    type Event = CloudEnvironmentCatalogEvent;
}

impl warpui::SingletonEntity for CloudEnvironmentCatalog {}

pub(crate) fn sort_environments_by_recency(environments: &mut [CloudAmbientAgentEnvironment]) {
    environments.sort_by(|a, b| {
        b.metadata
            .last_task_run_ts
            .cmp(&a.metadata.last_task_run_ts)
            .then_with(|| {
                a.model()
                    .string_model
                    .name
                    .to_lowercase()
                    .cmp(&b.model().string_model.name.to_lowercase())
            })
    });
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
