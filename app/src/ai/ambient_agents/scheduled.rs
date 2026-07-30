use std::collections::HashMap;
use std::future::Future;

pub use cloud_object_models::{
    CloudScheduledAmbientAgent, CloudScheduledAmbientAgentModel, ScheduledAmbientAgent,
};
use futures::FutureExt;
use futures::channel::oneshot;
use serde_json::{Map, Value};
use warp_graphql::queries::get_scheduled_agent_history::ScheduledAgentHistory;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use crate::cloud_object::model::generic_string_model::StringModel;
use crate::cloud_object::model::json_model::JsonModel;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{
    CloudObjectLookup as _, GenericStringObjectFormat, GenericStringObjectUniqueKey,
    JsonObjectType, Owner, Revision,
};
use crate::drive::CloudObjectTypeAndId;
use crate::server::cloud_objects::update_manager::{
    ObjectOperation, OperationSuccessType, UpdateManager, UpdateManagerEvent,
};
use crate::server::ids::{ClientId, SyncId};
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::QueueItem;

impl StringModel for ScheduledAmbientAgent {
    type CloudObjectType = CloudScheduledAmbientAgent;

    fn model_type_name(&self) -> &'static str {
        "Scheduled ambient agent"
    }

    fn should_enforce_revisions() -> bool {
        true
    }

    fn model_format() -> GenericStringObjectFormat {
        GenericStringObjectFormat::Json(JsonObjectType::ScheduledAmbientAgent)
    }

    fn display_name(&self) -> String {
        self.name.clone()
    }

    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &CloudScheduledAmbientAgent,
    ) -> QueueItem {
        QueueItem::UpdateScheduledAmbientAgent {
            model: object.model().clone().into(),
            id: object.id,
            revision: revision_ts.or_else(|| object.metadata.revision.clone()),
        }
    }

    fn uniqueness_key(&self) -> Option<GenericStringObjectUniqueKey> {
        None
    }

    fn should_show_activity_toasts() -> bool {
        false
    }

    fn warn_if_unsaved_at_quit() -> bool {
        true
    }
}

impl JsonModel for ScheduledAmbientAgent {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::ScheduledAmbientAgent
    }
}

/// Parameters for updating a scheduled ambient agent.
pub struct UpdateScheduleParams {
    /// The new name of the scheduled agent. If not provided, the name will not be updated.
    pub name: Option<String>,
    /// The new cron schedule of the scheduled agent. If not provided, the cron schedule will not be updated.
    pub cron: Option<String>,
    /// The new model ID of the scheduled agent. If not provided, the model ID will not be updated.
    pub model_id: Option<String>,
    /// The new environment ID of the scheduled agent.
    /// If this is:
    /// * `Some(Some(id))`, the environment ID will be updated to the given ID.
    /// * `Some(None)`, the environment will be removed.
    /// * `None`, the environment will not be updated.
    pub environment_id: Option<Option<String>>,
    /// The new base prompt to use for the scheduled agent's configuration.
    /// If not provided, the base prompt will not be updated.
    pub base_prompt: Option<String>,
    /// The new prompt of the scheduled agent. If not provided, the prompt will not be updated.
    pub prompt: Option<String>,
    /// MCP servers to upsert into this schedule's agent config.
    ///
    /// Entries are merged by key, overwriting existing keys.
    pub mcp_servers_upsert: Option<Map<String, Value>>,
    /// MCP server names (keys) to remove from this schedule's agent config.
    pub remove_mcp_server_names: Vec<String>,
    /// The new skill spec for the scheduled agent.
    /// If this is:
    /// * `Some(Some(spec))`, the skill spec will be updated to the given value.
    /// * `Some(None)`, the skill will be removed.
    /// * `None`, the skill spec will not be updated.
    pub skill_spec: Option<Option<String>>,
    /// The new worker host for the scheduled agent.
    /// If not provided, the worker host will not be updated.
    /// Setting to "warp" or empty string reverts to Warp-hosted.
    pub worker_host: Option<String>,
}

pub struct ScheduledAgentManager {
    /// Mapping of channels for schedules that are pending deletion, so that we can
    /// report when they complete from the CLI.
    pending_deletes: HashMap<SyncId, oneshot::Sender<anyhow::Result<()>>>,
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
impl ScheduledAgentManager {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &UpdateManager::handle(ctx),
            Self::handle_update_manager_event,
        );

        Self {
            pending_deletes: Default::default(),
        }
    }

    /// List all scheduled ambient agents currently present in the local cloud object store.
    pub fn list_schedules(&self, app: &AppContext) -> Vec<CloudScheduledAmbientAgent> {
        CloudScheduledAmbientAgent::get_all(app)
    }

    /// Get the execution history for a scheduled ambient agent.
    pub fn fetch_schedule_history(
        &self,
        schedule_id: SyncId,
        app: &AppContext,
    ) -> impl warpui::r#async::Spawnable<Output = anyhow::Result<Option<ScheduledAgentHistory>>> + use<>
    {
        let ai_client = ServerApiProvider::as_ref(app).get_ai_client();

        async move {
            let SyncId::ServerId(server_id) = schedule_id else {
                return Ok(None);
            };

            let schedule_id = server_id.to_string();
            let history = ai_client.get_scheduled_agent_history(&schedule_id).await?;
            Ok(Some(history))
        }
    }

    fn handle_update_manager_event(
        &mut self,
        _: ModelHandle<UpdateManager>,
        event: &UpdateManagerEvent,
        _ctx: &mut ModelContext<Self>,
    ) {
        if let UpdateManagerEvent::ObjectOperationComplete { result } = event
            && let ObjectOperation::Delete { .. } = result.operation
            && let Some(server_id) = result.server_id
        {
            let sync_id = SyncId::ServerId(server_id);
            if let Some(tx) = self.pending_deletes.remove(&sync_id) {
                match result.success_type {
                    OperationSuccessType::Success => {
                        let _ = tx.send(Ok(()));
                    }
                    OperationSuccessType::Failure => {
                        let _ = tx.send(Err(anyhow::anyhow!(
                            "Failed to delete scheduled ambient agent"
                        )));
                    }
                    OperationSuccessType::Denied(ref message) => {
                        let _ = tx.send(Err(anyhow::anyhow!("Deletion denied: {}", message)));
                    }
                    OperationSuccessType::Rejection => {
                        let _ = tx.send(Err(anyhow::anyhow!("Deletion rejected by server")));
                    }
                    OperationSuccessType::FeatureNotAvailable => {
                        let _ = tx.send(Err(anyhow::anyhow!(
                            "Scheduled ambient agents not available"
                        )));
                    }
                }
            }
        }
    }

    /// Create a new scheduled ambient agent.
    pub fn create_schedule(
        &mut self,
        config: ScheduledAmbientAgent,
        owner: Owner,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = anyhow::Result<SyncId>> + Send + 'static + use<> {
        let client_id = ClientId::default();
        let create_future = UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.create_scheduled_ambient_agent_online(config, client_id, owner, ctx)
        });
        async move { create_future.await.map(SyncId::ServerId) }
    }

    /// Helper method to fetch a schedule, modify its model, update it, and wait for completion.
    fn modify_schedule<F>(
        &mut self,
        schedule_id: SyncId,
        error_message: &'static str,
        modifier: F,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send + 'static + use<F>
    where
        F: FnOnce(&mut ScheduledAmbientAgent) + Send + 'static,
    {
        let schedule_object = CloudScheduledAmbientAgent::get_by_id(&schedule_id, ctx);

        match schedule_object {
            Some(schedule_obj) => {
                let mut updated_config = schedule_obj.model().string_model.clone();
                modifier(&mut updated_config);

                let revision = schedule_obj.metadata.revision.clone();

                let update_future =
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        update_manager.update_scheduled_ambient_agent_online(
                            updated_config,
                            schedule_id,
                            revision,
                            ctx,
                        )
                    });

                async move {
                    update_future
                        .await
                        .map_err(|e| anyhow::anyhow!("{}: {}", error_message, e))
                }
                .boxed()
            }
            None => async move { Err(anyhow::anyhow!("Schedule not found")) }.boxed(),
        }
    }

    /// Pause a scheduled ambient agent.
    pub fn pause_schedule(
        &mut self,
        schedule_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send + 'static + use<> {
        self.modify_schedule(
            schedule_id,
            "Failed to pause schedule",
            |config| config.enabled = false,
            ctx,
        )
    }

    /// Unpause a scheduled ambient agent.
    pub fn unpause_schedule(
        &mut self,
        schedule_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send + 'static + use<> {
        self.modify_schedule(
            schedule_id,
            "Failed to unpause schedule",
            |config| config.enabled = true,
            ctx,
        )
    }

    /// Update a scheduled ambient agent.
    pub fn update_schedule(
        &mut self,
        schedule_id: SyncId,
        params: UpdateScheduleParams,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send + 'static + use<> {
        self.modify_schedule(
            schedule_id,
            "Failed to update schedule",
            move |config| {
                if let Some(new_name) = params.name {
                    config.name = new_name;
                }
                if let Some(new_cron) = params.cron {
                    config.cron_schedule = new_cron;
                }

                if let Some(model_id) = params.model_id {
                    config.agent_config.model_id = Some(model_id);
                }

                if let Some(environment_id) = params.environment_id {
                    config.agent_config.environment_id = environment_id;
                }

                if let Some(base_prompt) = params.base_prompt {
                    config.agent_config.base_prompt = Some(base_prompt);
                }

                if let Some(prompt) = params.prompt {
                    config.prompt = prompt;
                }

                if let Some(skill_spec) = params.skill_spec {
                    config.agent_config.skill_spec = skill_spec;
                }

                if let Some(worker_host) = params.worker_host {
                    config.agent_config.worker_host = Some(worker_host);
                }

                for server_name in params.remove_mcp_server_names {
                    let server_name = server_name.trim();
                    if server_name.is_empty() {
                        continue;
                    }

                    if let Some(mcp_servers) = config.agent_config.mcp_servers.as_mut() {
                        mcp_servers.remove(server_name);
                    }
                }

                if let Some(upsert) = params.mcp_servers_upsert {
                    let mcp_servers = config.agent_config.mcp_servers.get_or_insert_with(Map::new);
                    for (name, config_value) in upsert {
                        mcp_servers.insert(name, config_value);
                    }
                }

                if config
                    .agent_config
                    .mcp_servers
                    .as_ref()
                    .is_some_and(|mcp_servers| mcp_servers.is_empty())
                {
                    config.agent_config.mcp_servers = None;
                }
            },
            ctx,
        )
    }

    /// Delete a scheduled ambient agent.
    pub fn delete_schedule(
        &mut self,
        schedule_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send + 'static + use<> {
        let id_and_type = CloudObjectTypeAndId::GenericStringObject {
            object_type: GenericStringObjectFormat::Json(JsonObjectType::ScheduledAmbientAgent),
            id: schedule_id,
        };

        let (tx, rx) = oneshot::channel();

        match CloudModel::as_ref(ctx).get_by_uid(&schedule_id.uid()) {
            None => {
                let _ = tx.send(Err(anyhow::anyhow!("Schedule {schedule_id} not found")));
            }
            Some(schedule) => {
                if schedule.metadata().has_pending_online_only_change()
                    || schedule.metadata().pending_changes_statuses.pending_delete
                {
                    let _ = tx.send(Err(anyhow::anyhow!(
                        "Cannot delete schedule with pending changes"
                    )));
                } else {
                    self.pending_deletes.insert(schedule_id, tx);
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        update_manager.delete_object_by_user(id_and_type, ctx);
                    });
                }
            }
        }

        async move {
            rx.await
                .map_err(|e| anyhow::anyhow!("Failed to delete schedule: {}", e))?
        }
    }
}

impl Entity for ScheduledAgentManager {
    type Event = ();
}

impl SingletonEntity for ScheduledAgentManager {}
