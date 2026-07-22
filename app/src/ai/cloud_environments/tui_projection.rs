use std::path::PathBuf;

use warpui::{Entity, ModelContext, SingletonEntity as _};

use super::CloudAmbientAgentEnvironment;
use crate::ai::blocklist::handoff::touched_repos::{
    TouchedWorkspace, pick_handoff_overlap_env, resolve_repo_for_path,
};
use crate::ai::orchestration::resolve_default_environment_id;
use crate::cloud_object::CloudObjectLookup as _;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ServerId, SyncId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuiCloudEnvironment {
    pub id: SyncId,
    pub name: String,
}

#[cfg(any(test, feature = "test-util"))]
impl TuiCloudEnvironment {
    pub fn new_for_test(id: i64, name: impl Into<String>) -> Self {
        Self {
            id: SyncId::ServerId(ServerId::from(id)),
            name: name.into(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TuiCloudEnvironmentEvent;

pub struct TuiCloudEnvironmentProjection {
    environments: Vec<TuiCloudEnvironment>,
}

impl TuiCloudEnvironmentProjection {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&CloudModel::handle(ctx), |projection, _, event, ctx| {
            match event {
                // `CloudModel::create_object` emits before inserting. Defer the
                // lookup until the source model update and event flush finish.
                CloudModelEvent::ObjectCreated { .. } => {
                    ctx.spawn(async {}, |projection, (), ctx| {
                        projection.refresh(false, ctx)
                    });
                }
                CloudModelEvent::InitialLoadCompleted => projection.refresh(true, ctx),
                CloudModelEvent::ObjectMoved { .. }
                | CloudModelEvent::ObjectUpdated { .. }
                | CloudModelEvent::ObjectTrashed { .. }
                | CloudModelEvent::ObjectUntrashed { .. }
                | CloudModelEvent::NotebookEditorChangedFromServer { .. }
                | CloudModelEvent::ObjectDeleted { .. }
                | CloudModelEvent::ObjectPermissionsUpdated { .. }
                | CloudModelEvent::ObjectForceExpanded { .. }
                | CloudModelEvent::ObjectSynced { .. } => projection.refresh(false, ctx),
            }
        });
        Self {
            environments: Self::current_environments(ctx),
        }
    }

    pub fn environments(&self) -> &[TuiCloudEnvironment] {
        &self.environments
    }

    pub fn refresh_from_server(&self, ctx: &mut ModelContext<Self>) {
        UpdateManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.refresh_updated_objects(ctx);
        });
    }

    pub fn default_environment_id(&self, ctx: &warpui::AppContext) -> Option<SyncId> {
        resolve_default_environment_id(ctx)
            .and_then(|id| ServerId::try_from(id.as_str()).ok())
            .map(SyncId::ServerId)
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn replace_for_test(
        &mut self,
        environments: Vec<TuiCloudEnvironment>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.environments = environments;
        ctx.emit(TuiCloudEnvironmentEvent);
        ctx.notify();
    }

    fn refresh(&mut self, force_emit: bool, ctx: &mut ModelContext<Self>) {
        let environments = Self::current_environments(ctx);
        if force_emit || environments != self.environments {
            self.environments = environments;
            ctx.emit(TuiCloudEnvironmentEvent);
            ctx.notify();
        }
    }

    fn current_environments(ctx: &warpui::AppContext) -> Vec<TuiCloudEnvironment> {
        let mut environments = CloudAmbientAgentEnvironment::get_all(ctx)
            .into_iter()
            .map(|environment| TuiCloudEnvironment {
                id: environment.id,
                name: environment.model().string_model.name.clone(),
            })
            .collect::<Vec<_>>();
        environments.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.to_string().cmp(&b.id.to_string()))
        });
        environments
    }
}

impl Entity for TuiCloudEnvironmentProjection {
    type Event = TuiCloudEnvironmentEvent;
}

pub fn suggest_tui_handoff_environment(
    path: PathBuf,
    ctx: &warpui::AppContext,
) -> impl std::future::Future<Output = Option<SyncId>> + Send + 'static {
    let environments = CloudAmbientAgentEnvironment::get_all(ctx);
    async move {
        let touched_repo = resolve_repo_for_path(&path).await?;
        pick_handoff_overlap_env(
            &TouchedWorkspace {
                repos: vec![touched_repo],
                orphan_files: Vec::new(),
            },
            environments,
        )
    }
}

#[cfg(test)]
#[path = "tui_projection_tests.rs"]
mod tests;
