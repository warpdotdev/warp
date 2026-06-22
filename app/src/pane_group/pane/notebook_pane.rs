use std::sync::Arc;

use anyhow::Context;
use url::Url;
use warpui::{AppContext, ModelHandle, SingletonEntity, ViewContext, ViewHandle};

use super::super::{DefaultSessionModeBehavior, Direction};
use super::view::PaneView;
use super::{
    DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};
use crate::app_state::{LeafContents, NotebookPaneSnapshot};
use crate::cloud_object::Space;
use crate::drive::items::WarpDriveItemId;
use crate::drive::{CloudObjectTypeAndId, OpenWarpDriveObjectSettings};
use crate::notebooks::link::{LinkEvent, NotebookLinks};
use crate::notebooks::manager::{NotebookManager, NotebookSource};
use crate::notebooks::notebook::{NotebookEvent, NotebookView};
use crate::server::ids::SyncId;
use crate::workflows::{WorkflowSelectionSource, WorkflowSource, WorkflowType};
use crate::workspaces::user_workspaces::UserWorkspaces;

pub struct NotebookPane {
    view: ViewHandle<PaneView<NotebookView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl NotebookPane {
    pub fn new(notebook_view: ViewHandle<NotebookView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = notebook_view.as_ref(ctx).pane_configuration().to_owned();
        let view = ctx.add_typed_action_view(notebook_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_notebook_pane_ctx(ctx);
            PaneView::new(pane_id, notebook_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    /// Restore a notebook pane given its cloud notebook ID.
    pub fn restore(
        notebook_id: Option<SyncId>,
        settings: &OpenWarpDriveObjectSettings,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> anyhow::Result<Self> {
        let window_id = ctx.window_id();
        let source = match notebook_id {
            Some(id) => NotebookSource::Existing(id),
            None => NotebookSource::New {
                title: None,
                owner: UserWorkspaces::as_ref(ctx)
                    .personal_drive(ctx)
                    .context("personal drive unavailable")?,
                initial_folder_id: None,
            },
        };

        Ok(NotebookManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.create_pane(&source, settings, window_id, ctx)
        }))
    }

    pub fn notebook_view(&self, ctx: &AppContext) -> ViewHandle<NotebookView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for NotebookPane {
    fn id(&self) -> PaneId {
        PaneId::from_notebook_pane_view(&self.view)
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        let notebook_id = self.notebook_view(app).as_ref(app).notebook_id(app);
        LeafContents::Notebook(NotebookPaneSnapshot::CloudNotebook {
            notebook_id,
            settings: OpenWarpDriveObjectSettings::default(),
        })
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        let pane_id = self.id();
        ctx.subscribe_to_view(&self.notebook_view(ctx), move |group, _, event, ctx| {
            handle_notebook_event(group, pane_id, event, ctx);
        });
        subscribe_to_link_model(pane_id, &self.notebook_view(ctx).as_ref(ctx).links(), ctx);

        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });

        let pane_group_id = ctx.view_id();
        let window_id = ctx.window_id();
        NotebookManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.register_pane(self, pane_group_id, window_id, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        // Always unsubscribe from views and models
        let notebook_view = self.notebook_view(ctx);
        ctx.unsubscribe_to_view(&notebook_view);
        ctx.unsubscribe_to_model(&notebook_view.as_ref(ctx).links());
        ctx.unsubscribe_to_view(&self.view);

        // Always deregister from NotebookManager - it will be re-registered on attach if restored
        NotebookManager::handle(ctx).update(ctx, |manager, ctx| manager.deregister_pane(self, ctx));

        self.notebook_view(ctx)
            .update(ctx, |view, ctx| view.on_detach(ctx));
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.notebook_view(ctx)
            .update(ctx, |view, ctx| view.focus(ctx));
    }

    fn shareable_link(
        &self,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        self.notebook_view(ctx).read(ctx, |view, ctx| {
            if let Some(link) = view.notebook_link(ctx) {
                if let Ok(parsed_url) = Url::parse(link.as_str()) {
                    Ok(ShareableLink::Pane { url: parsed_url })
                } else {
                    Err(ShareableLinkError::Unexpected(String::from(
                        "Failed to parse notebook url",
                    )))
                }
            } else {
                Err(ShareableLinkError::Unexpected(String::from(
                    "Could not retrieve notebook url from view",
                )))
            }
        })
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}

/// Subscribe to link events from a notebook view.
pub(super) fn subscribe_to_link_model(
    pane_id: PaneId,
    handle: &ModelHandle<NotebookLinks>,
    ctx: &mut ViewContext<PaneGroup>,
) {
    ctx.subscribe_to_model(handle, move |pane_group, _, event, ctx| match event {
        LinkEvent::OpenFileNotebook { path, session } => {
            // Opening local files is delegated to the parent workspace.
            ctx.emit(crate::pane_group::Event::OpenFileInWarp {
                path: crate::code::buffer_location::LocalOrRemotePath::Local(path.clone()),
                session: session.clone(),
            })
        }
        LinkEvent::OpenWarpDriveLink {
            open_warp_drive_args,
        } => ctx.emit(crate::pane_group::Event::OpenWarpDriveLink {
            open_warp_drive_args: open_warp_drive_args.clone(),
        }),
        LinkEvent::StartLocalSession { path } => {
            pane_group.add_session_in_directory(
                Direction::Right,
                Some(pane_id),
                None, /* chosen_shell */
                Some(path.clone()),
                None,
                DefaultSessionModeBehavior::Apply,
                ctx,
            );
        }
        #[cfg(feature = "local_fs")]
        LinkEvent::OpenFileWithTarget {
            path,
            target,
            line_col,
        } => {
            // Emit event to workspace to handle opening in Warp
            ctx.emit(crate::pane_group::Event::OpenFileWithTarget {
                path: path.clone(),
                target: target.clone(),
                line_col: *line_col,
            });
        }
        LinkEvent::RefreshLinks => (),
    });
}

/// Applies a notebook event to the containing pane group.
fn handle_notebook_event(
    group: &mut PaneGroup,
    pane_id: PaneId,
    event: &NotebookEvent,
    ctx: &mut ViewContext<PaneGroup>,
) {
    match event {
        NotebookEvent::RunWorkflow { workflow, source } => {
            run_notebook_workflow(workflow.clone(), *source, ctx)
        }
        NotebookEvent::EditWorkflow(id) => {
            ctx.emit(crate::pane_group::Event::OpenCloudWorkflowForEdit(*id))
        }
        NotebookEvent::ViewInWarpDrive(id) => view_in_warp_drive(*id, ctx),
        NotebookEvent::MoveToSpace {
            cloud_object_type_and_id,
            new_space,
        } => move_to_space(*cloud_object_type_and_id, *new_space, ctx),
        NotebookEvent::Pane(pane_event) => group.handle_pane_event(pane_id, pane_event, ctx),
        NotebookEvent::OpenDriveObjectShareDialog {
            cloud_object_type_and_id,
            invitee_email,
            source,
        } => ctx.emit(crate::pane_group::Event::OpenDriveObjectShareDialog {
            source: *source,
            cloud_object_type_and_id: *cloud_object_type_and_id,
            invitee_email: invitee_email.clone(),
        }),
        NotebookEvent::AttachPlanAsContext(ai_document_id) => {
            ctx.emit(crate::pane_group::Event::AttachPlanAsContext {
                ai_document_id: *ai_document_id,
            })
        }
    }
}

/// Runs a workflow from a notebook contained in this pane group in the active session.
fn run_notebook_workflow(
    workflow: Arc<WorkflowType>,
    workflow_source: WorkflowSource,
    ctx: &mut ViewContext<PaneGroup>,
) {
    // If the notebook was visible, then this pane group is almost certainly the active tab at the
    // workspace level. However, we dispatch to the workspace anyways for consistency (e.g. showing
    // a message if the active session is busy).
    ctx.emit(crate::pane_group::Event::RunWorkflow {
        workflow,
        workflow_source,
        workflow_selection_source: WorkflowSelectionSource::Notebook,
        argument_override: None,
    });
}

fn view_in_warp_drive(id: WarpDriveItemId, ctx: &mut ViewContext<PaneGroup>) {
    ctx.emit(crate::pane_group::Event::ViewInWarpDrive(id))
}

fn move_to_space(
    cloud_object_type_and_id: CloudObjectTypeAndId,
    space: Space,
    ctx: &mut ViewContext<PaneGroup>,
) {
    ctx.emit(crate::pane_group::Event::MoveToSpace {
        cloud_object_type_and_id,
        space,
    });
}
