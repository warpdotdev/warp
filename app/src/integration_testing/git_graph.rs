//! Integration-test helpers for the Git Graph panel.

use warpui::integration::{AssertionCallback, TestStep};
use warpui::{async_assert, App, ViewHandle, WindowId};

use crate::integration_testing::view_getters::workspace_view;
use crate::workspace::view::git_graph::GitGraphView;

/// Resolves the [`GitGraphView`] handle (workspace → left panel → git graph).
fn git_graph_view(app: &mut App, window_id: WindowId) -> ViewHandle<GitGraphView> {
    let workspace = workspace_view(app, window_id);
    let left_panel = workspace.read(app, |workspace, _| workspace.left_panel_view());
    left_panel.read(app, |left_panel, _| left_panel.git_graph_view())
}

/// A step that opens the Git Graph in the left panel.
pub fn open_git_graph_panel() -> TestStep {
    TestStep::new("Open the Git Graph panel").with_action(|app, window_id, _| {
        workspace_view(app, window_id).update(app, |workspace, ctx| {
            workspace.open_git_graph_panel(ctx);
        });
    })
}

/// Asserts the Git Graph has finished loading and shows at least one commit.
pub fn assert_git_graph_loaded() -> AssertionCallback {
    Box::new(|app, window_id| {
        let view = git_graph_view(app, window_id);
        view.read(app, |view, _| {
            async_assert!(
                view.is_loaded() && view.loaded_commit_count() > 0,
                "expected the Git Graph to be loaded with at least one commit"
            )
        })
    })
}
