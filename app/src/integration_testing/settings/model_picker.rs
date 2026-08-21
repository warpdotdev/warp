use warpui::{App, SingletonEntity, WindowId};

use crate::ai::execution_profiles::editor::ExecutionProfileEditorView;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::integration_testing::view_getters::workspace_view;
use crate::integration_testing::view_of_type;

/// Opens the execution profile editor pane for the default profile and expands
/// its base-model dropdown, so an integration test can capture the model
/// picker's rendered leading icons (provider logos, the Kimi logo, etc.) on
/// screen.
pub fn open_default_profile_base_model_dropdown(app: &mut App, window_id: WindowId) {
    let profile_id = app.read(|ctx| AIExecutionProfilesModel::as_ref(ctx).default_profile_id());

    workspace_view(app, window_id).update(app, |workspace, ctx| {
        workspace.open_execution_profile_editor_pane(None, profile_id, ctx);
    });

    let editor_view: warpui::ViewHandle<ExecutionProfileEditorView> =
        view_of_type(app, window_id, 0);
    editor_view.update(app, |view, ctx| {
        view.integration_test_toggle_base_model_dropdown(ctx);
    });
}
