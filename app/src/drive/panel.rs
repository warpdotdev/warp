use futures::Future;
use warpui::{
    elements::{Align, Flex, Hoverable, MouseStateHandle, ParentElement, SavePosition, Shrinkable},
    presenter::ChildView,
    windowing::{StateEvent, WindowManager},
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::{
    ai::{document::ai_document_model::AIDocumentId, facts::CloudAIFactModel},
    cloud_object::{
        model::{persistence::CloudModel, view::CloudViewModel},
        CloudObjectEventEntrypoint, GenericStringObjectFormat, JsonObjectType, Owner, Space,
    },
    env_vars::{manager::EnvVarCollectionSource, CloudEnvVarCollection},
    notebooks::{manager::NotebookSource, CloudNotebook},
    server::{
        cloud_objects::update_manager::{InitiatedBy, UpdateManager},
        ids::{ClientId, ServerId, SyncId},
        telemetry::SharingDialogSource,
    },
    workflows::{manager::WorkflowOpenSource, CloudWorkflow, WorkflowViewMode},
    workspaces::user_workspaces::UserWorkspaces,
};

use super::{
    drive_helpers::{
        has_feature_gated_anonymous_user_reached_env_var_limit,
        has_feature_gated_anonymous_user_reached_notebook_limit,
        has_feature_gated_anonymous_user_reached_workflow_limit,
    },
    index::{DriveIndex, DriveIndexAction, DriveIndexEvent},
    items::WarpDriveItemId,
    CloudObjectTypeAndId, DriveObjectType,
};

pub const MIN_SIDEBAR_WIDTH: f32 = 250.;
pub const MAX_SIDEBAR_WIDTH_RATIO: f32 = 0.75;

pub const WARP_DRIVE_POSITION_ID: &str = "warp_drive";

/// The sidebar that houses Warp Drive.
/// `DrivePanel` is different from `DriveIndex` in that it is responsible for
/// how Warp Drive interacts with the workspace and the rest of the app, whereas
/// `DriveIndex` is the main warp drive view and responsible for the internals of Warp Drive.
pub struct DrivePanel {
    index_view: ViewHandle<DriveIndex>,
    mouse_state_handles: MouseStateHandles,
}

#[derive(Clone, Default)]
struct MouseStateHandles {
    focus_panel_mouse_state: MouseStateHandle,
}

#[derive(Clone, Debug)]
pub enum DrivePanelAction {
    /// Open the search dialog.
    OpenSearch,
    /// Focus WD panel (via single click)
    FocusDriveIndex,
}

#[derive(Clone, Debug)]
pub enum DrivePanelEvent {
    RunWorkflow(Box<CloudWorkflow>),
    InvokeEnvironmentVariables {
        env_var_collection: Box<CloudEnvVarCollection>,
        in_subshell: bool,
    },
    OpenSearch,
    OpenSharedObjectsCreationDeniedModal(DriveObjectType, ServerId),
    OpenTeamSettingsPage,
    OpenAIFactCollection,
    OpenMCPServerCollection,
    OpenImportModal {
        owner: Owner,
        initial_folder_id: Option<SyncId>,
    },
    OpenWorkflowModalWithNew {
        space: Space,
        initial_folder_id: Option<SyncId>,
    },
    OpenWorkflowModalWithCloudWorkflow(SyncId),
    OpenNotebook(NotebookSource),
    OpenEnvVarCollection(EnvVarCollectionSource),
    OpenWorkflowInPane(WorkflowOpenSource, WorkflowViewMode),
    FocusWarpDrive,
    AttachPlanAsContext(AIDocumentId),
}

impl DrivePanel {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let index_view = ctx.add_typed_action_view(move |ctx| {
            let mut index = DriveIndex::new(ctx);
            index.initialize_section_states(ctx);
            index
        });
        ctx.subscribe_to_view(&index_view, |me, _, event, ctx| {
            me.handle_index_view_event(event, ctx);
        });

        // Subscribe to window state changes for focus dimming updates
        let state_handle = WindowManager::handle(ctx);
        ctx.subscribe_to_model(&state_handle, |_me, _, event, ctx| {
            match &event {
                StateEvent::ValueChanged { current, previous } => {
                    // Re-render if this window's focus state has changed
                    if WindowManager::did_window_change_focus(ctx.window_id(), current, previous) {
                        ctx.notify();
                    }
                }
            }
        });

        Self {
            index_view,
            mouse_state_handles: Default::default(),
        }
    }

    /// Helper to get the [`Owner`] for a new object created from the index.
    fn new_object_owner(
        space: Space,
        initial_folder_id: Option<&SyncId>,
        app: &AppContext,
    ) -> Option<Owner> {
        match initial_folder_id {
            Some(folder_id) => CloudModel::as_ref(app)
                .get_folder(folder_id)
                .map(|folder| folder.permissions.owner),
            None => UserWorkspaces::as_ref(app).space_to_owner(space, app),
        }
    }

    /// Event handler for actions that occur within the index view
    fn handle_index_view_event(&mut self, event: &DriveIndexEvent, ctx: &mut ViewContext<Self>) {
        match event {
            DriveIndexEvent::CreateNotebook {
                space,
                title,
                initial_folder_id,
            } => match Self::new_object_owner(*space, initial_folder_id.as_ref(), ctx) {
                Some(owner) => {
                    ctx.emit(DrivePanelEvent::OpenNotebook(NotebookSource::New {
                        title: title.clone(),
                        owner,
                        initial_folder_id: *initial_folder_id,
                    }));
                }
                None => {
                    log::error!("Cannot identify a notebook owner from {space:?}");
                }
            },
            DriveIndexEvent::OpenImportModal {
                space,
                initial_folder_id,
            } => match Self::new_object_owner(*space, initial_folder_id.as_ref(), ctx) {
                Some(owner) => ctx.emit(DrivePanelEvent::OpenImportModal {
                    owner,
                    initial_folder_id: *initial_folder_id,
                }),
                None => {
                    log::error!("Cannot identify an import target from {space:?}");
                }
            },
            DriveIndexEvent::CreateFolder {
                space,
                title,
                initial_folder_id,
            } => match Self::new_object_owner(*space, initial_folder_id.as_ref(), ctx) {
                Some(owner) => {
                    let client_id = ClientId::default();
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        update_manager.create_folder(
                            title.clone(),
                            owner,
                            client_id,
                            *initial_folder_id,
                            true,
                            InitiatedBy::User,
                            ctx,
                        );
                    });
                }
                None => {
                    log::error!("Cannot identify a folder owner from {space:?}");
                }
            },
            DriveIndexEvent::CreateEnvVarCollection {
                space,
                title,
                initial_folder_id,
            } => match Self::new_object_owner(*space, initial_folder_id.as_ref(), ctx) {
                Some(owner) => ctx.emit(DrivePanelEvent::OpenEnvVarCollection(
                    EnvVarCollectionSource::New {
                        title: title.clone(),
                        owner,
                        initial_folder_id: *initial_folder_id,
                    },
                )),
                None => {
                    log::error!("Cannot identify an env var owner from {space:?}");
                }
            },
            DriveIndexEvent::CreateWorkflow {
                space,
                title,
                initial_folder_id,
                is_for_agent_mode,
                content,
            } => match Self::new_object_owner(*space, initial_folder_id.as_ref(), ctx) {
                Some(owner) => ctx.emit(DrivePanelEvent::OpenWorkflowInPane(
                    WorkflowOpenSource::New {
                        title: title.clone(),
                        content: content.clone(),
                        owner,
                        initial_folder_id: *initial_folder_id,
                        is_for_agent_mode: *is_for_agent_mode,
                    },
                    WorkflowViewMode::Create,
                )),
                None => {
                    log::error!("Cannot identify a workflow owner from {space:?}");
                }
            },
            DriveIndexEvent::OpenAIFactCollection => {
                self.open_ai_fact_collection_pane(ctx);
            }
            DriveIndexEvent::OpenMCPServerCollection => {
                self.open_mcp_server_collection_pane(ctx);
            }
            DriveIndexEvent::OpenWorkflowInPane {
                cloud_object_type_and_id,
                open_mode,
            } => {
                let cloud_model = CloudModel::as_ref(ctx);
                let object = cloud_model.get_by_uid(&cloud_object_type_and_id.uid());

                let workflow: Option<&CloudWorkflow> = object.and_then(|object| object.into());
                if let Some(workflow) = workflow {
                    self.open_existing_workflow_in_pane(workflow.id, *open_mode, ctx);
                }
            }
            DriveIndexEvent::OpenObject(cloud_object_type_and_id) => {
                let cloud_model = CloudModel::as_ref(ctx);
                let object = cloud_model.get_by_uid(&cloud_object_type_and_id.uid());

                let notebook_id = object.and_then(|object| {
                    let notebook: Option<&CloudNotebook> = object.into();
                    notebook.map(|notebook| notebook.id)
                });

                let workflow: Option<&CloudWorkflow> = object.and_then(|object| object.into());

                let env_var_collection_id = object.and_then(|object| {
                    let env_var_collection: Option<&CloudEnvVarCollection> = object.into();
                    env_var_collection.map(|env_var_collection| env_var_collection.id)
                });

                if let Some(notebook_id) = notebook_id {
                    self.open_existing_notebook(notebook_id, ctx);
                } else if let Some(workflow) = workflow {
                    self.open_workflow_modal_with_existing(workflow.id, ctx);
                } else if let Some(env_var_collection_id) = env_var_collection_id {
                    self.open_existing_env_var_collection(env_var_collection_id, ctx);
                }
            }
            DriveIndexEvent::DuplicateObject(cloud_object_type_and_id) => {
                self.duplicate_object(cloud_object_type_and_id, ctx);
            }
            #[cfg(feature = "local_fs")]
            DriveIndexEvent::ExportObject(cloud_object_type_and_id) => {
                let window_id = ctx.window_id();
                super::export::ExportManager::handle(ctx).update(ctx, |export_manager, ctx| {
                    export_manager.export(window_id, &[*cloud_object_type_and_id], ctx);
                });
            }
            #[cfg(not(feature = "local_fs"))]
            DriveIndexEvent::ExportObject(_cloud_object_type_and_id) => {
                // No-op when no local filesystem.
            }
            DriveIndexEvent::OpenTeamSettingsPage => {
                ctx.emit(DrivePanelEvent::OpenTeamSettingsPage)
            }
            DriveIndexEvent::RunObject(id) => {
                let cloud_model = CloudModel::as_ref(ctx);
                let object = cloud_model.get_by_uid(&id.uid());
                if let Some(cloud_object) = object {
                    let workflow: Option<&CloudWorkflow> = cloud_object.into();
                    let env_var_collection: Option<&CloudEnvVarCollection> = cloud_object.into();
                    if let Some(workflow) = workflow {
                        self.run_workflow(workflow.clone(), ctx);
                    } else if let Some(env_var_collection) = env_var_collection {
                        self.invoke_environment_variables(env_var_collection.clone(), false, ctx);
                    }
                }
            }
            DriveIndexEvent::OpenWorkflowModalWithNew {
                space,
                initial_folder_id,
            } => self.open_workflow_modal_with_new(ctx, *space, *initial_folder_id),
            DriveIndexEvent::OpenWorkflowModalWithCloudWorkflow(workflow_id) => {
                self.open_workflow_modal_with_existing(*workflow_id, ctx)
            }
            DriveIndexEvent::FocusWarpDrive => ctx.emit(DrivePanelEvent::FocusWarpDrive),
            DriveIndexEvent::OpenSharedObjectsCreationDeniedModal(object_type, team_uid) => ctx
                .emit(DrivePanelEvent::OpenSharedObjectsCreationDeniedModal(
                    *object_type,
                    *team_uid,
                )),
            DriveIndexEvent::InvokeEnvVarCollectionInSubshell(id) => {
                let cloud_model = CloudModel::as_ref(ctx);
                let object = cloud_model.get_by_uid(&id.uid());
                if let Some(cloud_object) = object {
                    let env_var_collection: Option<&CloudEnvVarCollection> = cloud_object.into();
                    if let Some(env_var_collection) = env_var_collection {
                        self.invoke_environment_variables(env_var_collection.clone(), true, ctx);
                    }
                }
            }
            DriveIndexEvent::CreateAIFact {
                space,
                fact,
                initial_folder_id,
            } => match Self::new_object_owner(*space, initial_folder_id.as_ref(), ctx) {
                Some(owner) => {
                    let client_id = ClientId::default();
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        update_manager.create_object(
                            CloudAIFactModel::new(fact.clone()),
                            owner,
                            client_id,
                            CloudObjectEventEntrypoint::Blocklist,
                            true,
                            *initial_folder_id,
                            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                            // It can be changed to InitiatedBy::System if this action was automatically kicked off and does not require toasts to notify the user of completion.
                            InitiatedBy::User,
                            ctx,
                        );
                    });
                }
                None => {
                    log::error!("Cannot identify an AI rule owner from {space:?}");
                }
            },
            DriveIndexEvent::AttachPlanAsContext(id) => {
                ctx.emit(DrivePanelEvent::AttachPlanAsContext(*id))
            }
        }
    }

    fn duplicate_object(
        &mut self,
        cloud_object_type_and_id: &CloudObjectTypeAndId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Check if object being duplicated is in team space, if it is, then check
        // corresponding object limits for that team.
        if let Some(space) =
            CloudViewModel::as_ref(ctx).object_space(&cloud_object_type_and_id.uid(), ctx)
        {
            match space {
                Space::Team { team_uid } => {
                    match cloud_object_type_and_id {
                        CloudObjectTypeAndId::Notebook(_) => {
                            if !UserWorkspaces::has_capacity_for_shared_notebooks(team_uid, ctx, 1)
                            {
                                // If team has reached the limit for notebooks, show the modal
                                // and return early.
                                ctx.emit(DrivePanelEvent::OpenSharedObjectsCreationDeniedModal(
                                    DriveObjectType::Notebook {
                                        is_ai_document: false,
                                    },
                                    team_uid,
                                ));
                                return;
                            }
                        }
                        CloudObjectTypeAndId::Workflow(_) => {
                            if !UserWorkspaces::has_capacity_for_shared_workflows(team_uid, ctx, 1)
                            {
                                // If team has reached the limit for workflows, show the modal
                                // and return early.
                                ctx.emit(DrivePanelEvent::OpenSharedObjectsCreationDeniedModal(
                                    DriveObjectType::Workflow,
                                    team_uid,
                                ));
                                return;
                            }
                        }
                        _ => (),
                    }
                }
                Space::Personal => match cloud_object_type_and_id {
                    CloudObjectTypeAndId::Notebook(_) => {
                        if has_feature_gated_anonymous_user_reached_notebook_limit(ctx) {
                            return;
                        }
                    }
                    CloudObjectTypeAndId::Workflow(_) => {
                        if has_feature_gated_anonymous_user_reached_workflow_limit(ctx) {
                            return;
                        }
                    }
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type:
                            GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection),
                        id: _,
                    } => {
                        if has_feature_gated_anonymous_user_reached_env_var_limit(ctx) {
                            return;
                        }
                    }
                    _ => {}
                },
                // We're reliant on server checks here, since the client doesn't know how many
                // objects are in the owning drive.
                Space::Shared => (),
            }
        }

        // Otherwise allow object duplication to go through.
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.duplicate_object(cloud_object_type_and_id, ctx);
        });
        ctx.notify();
    }

    /// Sets the child view back to a default state
    fn save_and_clear_child_view(&mut self, ctx: &mut ViewContext<Self>) {
        self.reset_all_menus(ctx);
    }

    /// Reset all context menus in all views
    fn reset_all_menus(&mut self, ctx: &mut ViewContext<Self>) {
        self.index_view.update(ctx, |index_view, ctx| {
            index_view.reset_and_open_to_main_index(ctx);
            index_view.reset_menus(ctx);
        });
    }

    pub fn move_object_to_team_owner(
        &mut self,
        cloud_object_type_and_id: CloudObjectTypeAndId,
        space: Space,
        ctx: &mut ViewContext<Self>,
    ) {
        self.index_view.update(ctx, |index_view, ctx| {
            index_view.move_object_to_team_owner(&cloud_object_type_and_id, space, ctx);
        })
    }

    pub fn set_selected_object(
        &mut self,
        id: Option<WarpDriveItemId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.index_view.update(ctx, |index_view, ctx| {
            index_view.set_selected_object(id, ctx);
        });
    }

    pub fn run_workflow(&mut self, workflow: CloudWorkflow, ctx: &mut ViewContext<Self>) {
        ctx.emit(DrivePanelEvent::RunWorkflow(Box::new(workflow)));
        ctx.notify();
    }

    pub fn invoke_environment_variables(
        &mut self,
        env_var_collection: CloudEnvVarCollection,
        in_subshell: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(DrivePanelEvent::InvokeEnvironmentVariables {
            env_var_collection: Box::new(env_var_collection),
            in_subshell,
        });
        ctx.notify();
    }

    pub fn open_existing_workflow_in_pane(
        &self,
        workflow_id: SyncId,
        open_mode: WorkflowViewMode,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(DrivePanelEvent::OpenWorkflowInPane(
            WorkflowOpenSource::Existing(workflow_id),
            open_mode,
        ));
        ctx.notify();
    }

    pub fn open_workflow_modal_with_new(
        &mut self,
        ctx: &mut ViewContext<Self>,
        space: Space,
        initial_folder_id: Option<SyncId>,
    ) {
        ctx.emit(DrivePanelEvent::OpenWorkflowModalWithNew {
            space,
            initial_folder_id,
        });
    }

    pub fn open_existing_notebook(&self, notebook_id: SyncId, ctx: &mut ViewContext<Self>) {
        ctx.emit(DrivePanelEvent::OpenNotebook(NotebookSource::Existing(
            notebook_id,
        )));
        ctx.notify();
    }

    pub fn open_workflow_modal_with_existing(
        &mut self,
        workflow_id: SyncId,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(DrivePanelEvent::OpenWorkflowModalWithCloudWorkflow(
            workflow_id,
        ));
        ctx.notify();
    }

    pub fn open_existing_env_var_collection(
        &mut self,
        env_var_collection_id: SyncId,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(DrivePanelEvent::OpenEnvVarCollection(
            EnvVarCollectionSource::Existing(env_var_collection_id),
        ));
        ctx.notify();
    }

    pub fn open_cloud_object_dialog(
        &mut self,
        cloud_object_type: DriveObjectType,
        space: Space,
        initial_folder_id: Option<SyncId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.save_and_clear_child_view(ctx);
        self.index_view.update(ctx, |index_view, ctx| {
            index_view.handle_action(
                &DriveIndexAction::create_object(cloud_object_type, space, initial_folder_id),
                ctx,
            )
        });
        ctx.notify();
    }

    pub fn create_workflow_with_content(
        &mut self,
        space: Space,
        initial_folder_id: Option<SyncId>,
        content: String,
        is_for_agent_mode: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.save_and_clear_child_view(ctx);
        self.index_view.update(ctx, |index_view, ctx| {
            index_view.handle_action(
                &DriveIndexAction::CreateWorkflowWithContent {
                    space,
                    initial_folder_id,
                    content,
                    is_for_agent_mode,
                },
                ctx,
            )
        });
        ctx.notify();
    }

    pub fn open_ai_fact_collection_pane(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(DrivePanelEvent::OpenAIFactCollection);
    }

    pub fn open_mcp_server_collection_pane(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(DrivePanelEvent::OpenMCPServerCollection);
    }

    /// Recomputes and initializes the section states for the WD Index. This is needed after
    /// we directly change anything about the state of the index (such as folders being open/closed).
    ///
    /// This should only be called if we immeidiately need to update and rely on the updated state.
    pub fn initialize_drive_section_states(&mut self, ctx: &mut ViewContext<Self>) {
        self.index_view.update(ctx, |index, ctx| {
            index.initialize_section_states(ctx);
        })
    }

    pub fn expand_section_for_drive_item_id(
        &mut self,
        item_id: WarpDriveItemId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.index_view.update(ctx, |index, ctx| {
            index.expand_section_for_drive_item_id(item_id, ctx);
        })
    }

    /// This functions scrolls the relevant Warp Drive item into view.
    pub fn scroll_item_into_view(&mut self, item_id: WarpDriveItemId, ctx: &mut ViewContext<Self>) {
        self.index_view.update(ctx, |index, ctx| {
            index.scroll_item_into_view(item_id, ctx);
        })
    }

    /// This functions sets the index of a focused Warp Drive item.
    pub fn set_focused_index(&mut self, focused_index: Option<usize>, ctx: &mut ViewContext<Self>) {
        self.index_view.update(ctx, |index, ctx| {
            index.set_focused_index(focused_index, true, ctx);
        })
    }

    pub fn set_focused_item(&mut self, item_id: WarpDriveItemId, ctx: &mut ViewContext<Self>) {
        self.index_view.update(ctx, |index, ctx| {
            ctx.focus(&self.index_view);
            index.set_focused_item(item_id, true, ctx);
        })
    }

    pub fn open_object_sharing_settings(
        &mut self,
        object_id: CloudObjectTypeAndId,
        invitee_email: Option<String>,
        source: SharingDialogSource,
        ctx: &mut ViewContext<Self>,
    ) {
        let warp_drive_item_id = WarpDriveItemId::Object(object_id);
        self.index_view.update(ctx, |index, ctx| {
            index.set_focused_item(warp_drive_item_id, true, ctx);
            index.toggle_share_dialog(&warp_drive_item_id, invitee_email, source, ctx);
        });
    }

    pub fn has_warp_drive_initialized_sections(
        &self,
        app: &AppContext,
    ) -> impl Future<Output = ()> {
        self.index_view.as_ref(app).has_initialized_sections()
    }

    pub fn reset_focused_index_in_warp_drive(
        &mut self,
        should_scroll: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.index_view.update(ctx, |index, ctx| {
            index.reset_focused_index_in_warp_drive(should_scroll, ctx);
        })
    }

    pub fn reset_and_open_to_main_index(&mut self, ctx: &mut ViewContext<Self>) {
        self.index_view.update(ctx, |index, ctx| {
            index.reset_and_open_to_main_index(ctx);
        })
    }

    pub fn undo_trash(
        &mut self,
        cloud_object_type_and_id: &CloudObjectTypeAndId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.index_view.update(ctx, |index_view, ctx| {
            index_view.untrash_object(cloud_object_type_and_id, ctx)
        });
    }
}

impl View for DrivePanel {
    fn ui_name() -> &'static str {
        "WarpDrivePanel"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.index_view);
        }
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let body = Hoverable::new(
            self.mouse_state_handles.focus_panel_mouse_state.clone(),
            |_| {
                Align::new(
                    SavePosition::new(
                        ChildView::new(&self.index_view).finish(),
                        WARP_DRIVE_POSITION_ID,
                    )
                    .finish(),
                )
                .top_center()
                .finish()
            },
        )
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(DrivePanelAction::FocusDriveIndex);
        })
        .finish();

        let mut col = Flex::column();
        col.add_child(Shrinkable::new(1., body).finish());
        col.with_main_axis_size(warpui::elements::MainAxisSize::Max)
            .finish()
    }
}

impl Entity for DrivePanel {
    type Event = DrivePanelEvent;
}

impl TypedActionView for DrivePanel {
    type Action = DrivePanelAction;

    fn handle_action(&mut self, action: &DrivePanelAction, ctx: &mut ViewContext<Self>) {
        match action {
            DrivePanelAction::OpenSearch => ctx.emit(DrivePanelEvent::OpenSearch),
            DrivePanelAction::FocusDriveIndex => {
                ctx.focus(&self.index_view);
                // should_scroll is set to false here in order to not let menu clicks autoscroll WD index
                self.reset_focused_index_in_warp_drive(false, ctx);
            }
        }
    }
}

pub(crate) mod styles {
    /// Right padding between the search button and the close button.
    pub const SEARCH_BUTTON_PADDING_RIGHT: f32 = 4.;
}

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
