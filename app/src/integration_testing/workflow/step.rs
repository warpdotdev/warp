use warpui::integration::TestStep;
use warpui::windowing::WindowManager;
use warpui::{SingletonEntity, WindowId, async_assert};

use super::open_workflow_count;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectEventEntrypoint, Space};
use crate::drive::OpenWarpDriveObjectSettings;
use crate::integration_testing::view_getters::workspace_view;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ClientId, SyncId};
use crate::workflows::WorkflowViewMode;
use crate::workflows::manager::WorkflowOpenSource;
use crate::workflows::workflow::Workflow;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Create a personal workflow and save its sync ID into the step data.
pub fn create_a_personal_workflow(key: impl Into<String>) -> TestStep {
    let key = key.into();
    let workflow = Workflow::new("personal workflow", "echo 'name'");
    TestStep::new("Create a personal workflow")
        .with_action(move |app, _, data| {
            let client_id = ClientId::new();
            let sync_id = SyncId::ClientId(client_id);
            UpdateManager::handle(app).update(app, |update_manager, ctx| {
                update_manager.create_workflow(
                    workflow.clone(),
                    UserWorkspaces::as_ref(ctx)
                        .personal_drive(ctx)
                        .expect("User UID must be set in tests"),
                    None,
                    client_id,
                    CloudObjectEventEntrypoint::ManagementUI,
                    true,
                    ctx,
                );
            });

            data.insert(key.clone(), sync_id);
        })
        .add_assertion(move |app, _| {
            CloudModel::handle(app).read(app, |cloud_model, ctx| {
                async_assert!(
                    cloud_model
                        .active_cloud_objects_in_space(Space::Personal, ctx)
                        .count()
                        > 0,
                    "Workflow exists"
                )
            })
        })
}

/// Open the workflow saved at `workflow_key` in the active tab of the window saved at `window_key`
pub fn open_workflow(window_key: impl Into<String>, workflow_key: impl Into<String>) -> TestStep {
    let window_key = window_key.into();
    let workflow_key = workflow_key.into();

    let workflow_other_key = workflow_key.clone();
    TestStep::new("Open workflow")
        .with_action(move |app, _, data| {
            let workflow_id: &SyncId = data.get(&workflow_key).expect("No saved workflow ID");
            let window_id: &WindowId = data.get(&window_key).expect("No saved window ID");
            workspace_view(app, *window_id).update(app, |workspace, ctx| {
                // If the workflow isn't open yet, opening it won't focus the window (we only change
                // focus if switching to an already-open window). Since the user wouldn't be able to
                // open a workflow in an unfocused window, switch focus explicitly here.
                WindowManager::as_ref(ctx).show_window_and_focus_app(*window_id);
                workspace.open_workflow_in_pane(
                    &WorkflowOpenSource::Existing(*workflow_id),
                    &OpenWarpDriveObjectSettings::default(),
                    WorkflowViewMode::View,
                    ctx,
                );
            })
        })
        .add_named_assertion_with_data_from_prior_step(
            "Check workflow is open",
            move |app, _, data| {
                let workflow_id: &SyncId =
                    data.get(&workflow_other_key).expect("No workflow ID found");
                async_assert!(open_workflow_count(app, *workflow_id) == 1)
            },
        )
}
