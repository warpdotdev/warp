use std::collections::HashMap;
use std::collections::hash_map::Entry;

use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use super::CloudWorkflowModel;
use super::workflow::Workflow;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{GenericCloudObject, Owner};
use crate::drive::OpenWarpDriveObjectSettings;
use crate::pane_group::{PaneContent, WorkflowPane};
use crate::server::cloud_objects::update_manager::{
    ObjectOperation, OperationSuccessType, UpdateManager, UpdateManagerEvent,
};
use crate::server::ids::{ClientId, SyncId};
use crate::workflows::WorkflowViewMode;
use crate::workflows::workflow_view::WorkflowView;
use crate::{PaneViewLocator, WindowId, safe_warn};

pub struct WorkflowManager {
    panes_by_hashed_id: HashMap<String, WorkflowPaneData>,
}

#[derive(Debug, Clone)]
pub enum WorkflowOpenSource {
    Existing(SyncId),
    New {
        title: Option<String>,

        /// The "content" of the workflow.
        /// For `Command` workflows, this is the command.
        /// For `AgentMode` workflows, this is the AI query.
        content: Option<String>,

        owner: Owner,
        initial_folder_id: Option<SyncId>,
        is_for_agent_mode: bool,
    },
    NewFromWorkflow {
        workflow: Box<Workflow>,
        owner: Owner,
        initial_folder_id: Option<SyncId>,
    },
}

impl WorkflowManager {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &UpdateManager::handle(ctx),
            Self::handle_update_manager_event,
        );

        WorkflowManager {
            panes_by_hashed_id: HashMap::new(),
        }
    }

    pub fn find_pane(&self, source: &WorkflowOpenSource) -> Option<(WindowId, PaneViewLocator)> {
        match source {
            WorkflowOpenSource::Existing(workflow_id) => {
                let pane_data = self.panes_by_hashed_id.get(&workflow_id.uid())?;
                Some((pane_data.window_id, pane_data.locator))
            }
            WorkflowOpenSource::New { .. } | WorkflowOpenSource::NewFromWorkflow { .. } => None,
        }
    }

    pub fn create_pane(
        &mut self,
        source: &WorkflowOpenSource,
        settings: &OpenWarpDriveObjectSettings,
        mode: WorkflowViewMode,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) -> WorkflowPane {
        let view = ctx.add_typed_action_view(window_id, WorkflowView::new_in_pane);

        match source {
            WorkflowOpenSource::Existing(workflow_id) => {
                let workflow = CloudModel::as_ref(ctx).get_workflow(workflow_id).cloned();
                if let Some(workflow) = workflow {
                    view.update(ctx, |view, ctx| view.load(workflow, settings, mode, ctx));
                } else {
                    // If the workflow doesn't exist, try waiting for initial load and trying again
                    view.update(ctx, |view, ctx| {
                        view.wait_for_initial_load_then_load(
                            *workflow_id,
                            settings,
                            mode,
                            window_id,
                            ctx,
                        )
                    });
                }
            }
            WorkflowOpenSource::New {
                title,
                content,
                owner,
                initial_folder_id,
                is_for_agent_mode,
            } => view.update(ctx, |view, ctx| {
                view.open_new_workflow(
                    title.clone(),
                    content.clone(),
                    *owner,
                    *initial_folder_id,
                    *is_for_agent_mode,
                    SyncId::ClientId(ClientId::default()),
                    ctx,
                )
            }),
            WorkflowOpenSource::NewFromWorkflow {
                workflow,
                owner,
                initial_folder_id,
            } => {
                view.update(ctx, |view, ctx| {
                    view.load(
                        GenericCloudObject::new_local(
                            CloudWorkflowModel::new(*workflow.clone()),
                            *owner,
                            *initial_folder_id,
                            ClientId::default(),
                        ),
                        &OpenWarpDriveObjectSettings::default(),
                        mode,
                        ctx,
                    );
                });
            }
        }

        WorkflowPane::new(view, ctx)
    }

    pub fn register_pane(
        &mut self,
        pane: &WorkflowPane,
        pane_group_id: EntityId,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) {
        let workflow_id = pane.get_view(ctx).as_ref(ctx).workflow_id();
        let entry = self.panes_by_hashed_id.entry(workflow_id.uid());
        if let Entry::Vacant(entry) = entry {
            entry.insert(WorkflowPaneData {
                workflow_id,
                window_id,
                locator: PaneViewLocator {
                    pane_group_id,
                    pane_id: pane.id(),
                },
            });
        } else {
            safe_warn!(
                safe: ("Ignoring duplicate Workflow pane registration"),
                full: ("Ignoring duplicate Workflow pane registration for {workflow_id}")
            );
        }
    }

    pub fn deregister_pane(&mut self, pane: &WorkflowPane, ctx: &mut ModelContext<Self>) {
        let workflow_id = pane.get_view(ctx).as_ref(ctx).workflow_id();

        // If a workflow pane is restored, the workflow may have been reopened in the meantime. In
        // that case, don't let closing the original pane clear out the new pane.
        if let Entry::Occupied(entry) = self.panes_by_hashed_id.entry(workflow_id.uid()) {
            if entry.get().locator.pane_id == pane.id() {
                entry.remove();
            } else {
                log::warn!(
                    "Ignoring duplicate registration of panes for {}",
                    workflow_id.uid()
                );
            }
        }
    }

    fn handle_update_manager_event(
        &mut self,
        _: ModelHandle<UpdateManager>,
        event: &UpdateManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let UpdateManagerEvent::ObjectOperationComplete { result } = event else {
            return;
        };

        if !matches!(&result.success_type, OperationSuccessType::Success) {
            return;
        }
        if let ObjectOperation::Create { .. } = result.operation {
            let server_id = result.server_id.expect("Expect server id on success");
            let Some(server_id) = CloudModel::as_ref(ctx)
                .get_workflow_by_uid(&server_id.uid())
                .and_then(|workflow| workflow.id.into_server())
            else {
                return;
            };
            let Some(client_id) = result.client_id else {
                return;
            };

            if let Some(mut pane) = self.panes_by_hashed_id.remove(&client_id.to_string()) {
                pane.workflow_id = SyncId::ServerId(server_id);
                self.panes_by_hashed_id
                    .insert(server_id.uid().clone(), pane);
            }
        }
    }

    pub fn reset(&mut self) {
        self.panes_by_hashed_id.clear();
    }
}

struct WorkflowPaneData {
    workflow_id: SyncId,
    window_id: WindowId,
    locator: PaneViewLocator,
}

impl Entity for WorkflowManager {
    type Event = ();
}

impl SingletonEntity for WorkflowManager {}
