use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use url::Url;
use warpui::{AppContext, ModelHandle, SingletonEntity, ViewContext, ViewHandle};

use super::{
    DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, PaneView, ShareableLink,
    ShareableLinkError,
};
use crate::app_state::{LeafContents, WorkflowPaneSnapshot};
use crate::drive::items::WarpDriveItemId;
use crate::drive::OpenWarpDriveObjectSettings;
use crate::server::ids::SyncId;
use crate::workflows::manager::{WorkflowManager, WorkflowOpenSource};
use crate::workflows::workflow_view::{WorkflowView, WorkflowViewEvent};
use crate::workflows::{WorkflowSelectionSource, WorkflowSource, WorkflowType, WorkflowViewMode};
use crate::workspaces::user_workspaces::UserWorkspaces;

pub struct WorkflowPane {
    view: ViewHandle<PaneView<WorkflowView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl WorkflowPane {
    pub fn new(view: ViewHandle<WorkflowView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = view.as_ref(ctx).pane_configuration().to_owned();
        let view = ctx.add_typed_action_view(view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_workflow_pane_ctx(ctx);
            PaneView::new(pane_id, view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    pub fn restore(
        workflow_id: Option<SyncId>,
        settings: OpenWarpDriveObjectSettings,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> anyhow::Result<Self> {
        let window_id = ctx.window_id();
        let source = match workflow_id {
            Some(id) => WorkflowOpenSource::Existing(id),
            None => WorkflowOpenSource::New {
                title: None,
                content: None,
                owner: UserWorkspaces::as_ref(ctx)
                    .personal_drive(ctx)
                    .context("personal drive unavailable")?,
                initial_folder_id: None,
                is_for_agent_mode: false,
            },
        };

        // default to view mode on restore -- feels safer
        Ok(WorkflowManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.create_pane(
                &source,
                &settings,
                WorkflowViewMode::supported_view_mode(workflow_id, ctx),
                window_id,
                ctx,
            )
        }))
    }

    pub fn get_view(&self, ctx: &AppContext) -> ViewHandle<WorkflowView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for WorkflowPane {
    fn id(&self) -> PaneId {
        PaneId::from_workflow_pane_view(&self.view)
    }

    /// Callback for when this leaf pane is added to a pane group.
    ///
    /// This is called after the pane is added to the group's set of leaf panes, but before the
    /// new pane is focused.
    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        let pane_id = self.id();
        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });

        ctx.subscribe_to_view(&self.get_view(ctx), move |group, _, event, ctx| {
            handle_workflow_event(group, pane_id, event, ctx);
        });

        let pane_group_id = ctx.view_id();
        let window_id = ctx.window_id();
        WorkflowManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.register_pane(self, pane_group_id, window_id, ctx);
        });
    }

    /// Callback for when this leaf pane is removed from a pane group.
    ///
    /// This is called when:
    /// - The pane is about to be closed
    /// - The pane group is closed, but may be restored
    /// - The pane is being moved to another tab, or upgraded to its own tab
    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        // Always unsubscribe from views
        ctx.unsubscribe_to_view(&self.view);
        ctx.unsubscribe_to_view(&self.get_view(ctx));

        // Always deregister from WorkflowManager - it will be re-registered on attach if restored
        WorkflowManager::handle(ctx).update(ctx, |manager, ctx| manager.deregister_pane(self, ctx));
    }

    /// Snapshot this pane for session restoration.
    fn snapshot(&self, app: &AppContext) -> LeafContents {
        let workflow_id = self.get_view(app).as_ref(app).workflow_id();
        LeafContents::Workflow(WorkflowPaneSnapshot::CloudWorkflow {
            workflow_id: Some(workflow_id),
            settings: OpenWarpDriveObjectSettings::default(),
        })
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    /// Focus this pane's contents.
    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.get_view(ctx).update(ctx, |view, ctx| view.focus(ctx));
    }

    fn shareable_link(
        &self,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        self.get_view(ctx).read(ctx, |view, ctx| {
            if let Some(link) = view.workflow_link(ctx) {
                if let Ok(parsed_url) = Url::parse(link.as_str()) {
                    Ok(ShareableLink::Pane { url: parsed_url })
                } else {
                    Err(ShareableLinkError::Unexpected(String::from(
                        "Failed to parse workflow url",
                    )))
                }
            } else {
                Err(ShareableLinkError::Unexpected(String::from(
                    "Could not retrieve workflow url from view",
                )))
            }
        })
    }

    /// Pane-agnostic state that all panes have.
    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}

fn handle_workflow_event(
    group: &mut PaneGroup,
    pane_id: PaneId,
    event: &WorkflowViewEvent,
    ctx: &mut ViewContext<PaneGroup>,
) {
    match event {
        WorkflowViewEvent::Pane(pane_event) => group.handle_pane_event(pane_id, pane_event, ctx),
        WorkflowViewEvent::ViewInWarpDrive(id) => view_in_warp_drive(*id, ctx),
        WorkflowViewEvent::RunWorkflow {
            workflow,
            source,
            argument_override,
        } => run_workflow(workflow.clone(), *source, argument_override.clone(), ctx),
        WorkflowViewEvent::UpdatedWorkflow(_id) => {
            log::warn!("Updates not yet handled in pane")
        }
        WorkflowViewEvent::OpenDriveObjectShareDialog {
            cloud_object_type_and_id,
            invitee_email,
            source,
        } => {
            ctx.emit(crate::pane_group::Event::OpenDriveObjectShareDialog {
                cloud_object_type_and_id: *cloud_object_type_and_id,
                invitee_email: invitee_email.clone(),
                source: *source,
            });
        }
        WorkflowViewEvent::CreatedWorkflow(_) => {
            // No op in a pane.
        }
    }
}

fn run_workflow(
    workflow: Arc<WorkflowType>,
    workflow_source: WorkflowSource,
    argument_override: Option<HashMap<String, String>>,
    ctx: &mut ViewContext<PaneGroup>,
) {
    ctx.emit(crate::pane_group::Event::RunWorkflow {
        workflow,
        workflow_source,
        argument_override,
        workflow_selection_source: WorkflowSelectionSource::WorkflowView,
    });
}

fn view_in_warp_drive(id: WarpDriveItemId, ctx: &mut ViewContext<PaneGroup>) {
    ctx.emit(crate::pane_group::Event::ViewInWarpDrive(id))
}
