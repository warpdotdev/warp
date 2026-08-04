//! Shared projection of cloud environments for GUI and TUI consumers.

use settings::Setting as _;
use warp_errors::report_if_error;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity as _};

use super::CloudAmbientAgentEnvironment;
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::cloud_object::CloudObjectLookup as _;
use crate::cloud_object::model::generic_string_model::StringModel as _;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
#[cfg(feature = "tui")]
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
    orchestration_default_environment_id: Option<SyncId>,
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
                | CloudModelEvent::EnvironmentLastTaskRunTimestampsUpdated
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
        let (environments, orchestration_default_environment_id) = Self::current_environments(ctx);
        Self {
            environments,
            orchestration_default_environment_id,
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
        self.saved_environment_id(ctx)
            .or_else(|| self.environments.first().map(|environment| environment.id))
    }

    /// Returns the saved environment when it still exists, otherwise the
    /// orchestration GUI's case-sensitive name fallback.
    pub fn orchestration_default_environment_id(&self, ctx: &AppContext) -> Option<SyncId> {
        self.saved_environment_id(ctx)
            .or(self.orchestration_default_environment_id)
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
    #[cfg(feature = "tui")]
    pub fn refresh_from_server(&self, ctx: &mut ModelContext<Self>) {
        UpdateManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.refresh_updated_objects(ctx);
        });
    }

    fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        let (environments, orchestration_default_environment_id) = Self::current_environments(ctx);
        if environments != self.environments
            || orchestration_default_environment_id != self.orchestration_default_environment_id
        {
            self.environments = environments;
            self.orchestration_default_environment_id = orchestration_default_environment_id;
            ctx.emit(CloudEnvironmentCatalogEvent);
            ctx.notify();
        }
    }

    fn saved_environment_id(&self, ctx: &AppContext) -> Option<SyncId> {
        (*CloudAgentSettings::as_ref(ctx)
            .last_selected_environment_id
            .value())
        .filter(|id| self.environment(*id).is_some())
    }

    fn current_environments(ctx: &AppContext) -> (Vec<CloudEnvironment>, Option<SyncId>) {
        let mut environments = CloudAmbientAgentEnvironment::get_all(ctx);
        let orchestration_default_environment_id = {
            let mut orchestration_environments = environments.clone();
            sort_environments_for_orchestration_default(&mut orchestration_environments);
            orchestration_environments
                .first()
                .map(|environment| environment.id)
        };
        sort_environments_by_recency(&mut environments);
        let environments = environments
            .into_iter()
            .map(|environment| CloudEnvironment {
                id: environment.id,
                name: environment.model().string_model.display_name(),
            })
            .collect();
        (environments, orchestration_default_environment_id)
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

fn sort_environments_for_orchestration_default(environments: &mut [CloudAmbientAgentEnvironment]) {
    environments.sort_by(|a, b| {
        b.metadata
            .last_task_run_ts
            .cmp(&a.metadata.last_task_run_ts)
            .then_with(|| {
                a.model()
                    .string_model
                    .name
                    .cmp(&b.model().string_model.name)
            })
    });
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
