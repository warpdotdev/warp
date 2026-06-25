//! Integration-test helpers for the Git Graph panel.

use warpui::integration::{AssertionCallback, TestStep};
use warpui::{async_assert, App, TypedActionView, ViewHandle, WindowId};

use crate::integration_testing::view_getters::workspace_view;
use crate::workspace::view::git_graph::ops::GitWriteOp;
use crate::workspace::view::git_graph::view::GitGraphAction;
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

/// A step that runs the "Create Branch" write op at the newest commit, exactly
/// as the context-menu action would. Exercises the full write path
/// (op → `git branch` → reload).
pub fn create_branch_at_top(name: &'static str) -> TestStep {
    TestStep::new("Create a branch at the top commit via the Git Graph").with_action(
        move |app, window_id, _| {
            let view = git_graph_view(app, window_id);
            view.update(app, |view, ctx| {
                if let Some(hash) = view.first_commit_hash_for_test() {
                    view.handle_action(
                        &GitGraphAction::RunOp(GitWriteOp::CreateBranch {
                            hash,
                            name: name.to_string(),
                        }),
                        ctx,
                    );
                }
            });
        },
    )
}

/// Asserts a local branch named `name` is known to the panel (i.e. the write op
/// took effect and the subsequent reload picked it up).
pub fn assert_local_branch_exists(name: &'static str) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let view = git_graph_view(app, window_id);
        view.read(app, |view, _| {
            async_assert!(
                view.has_local_branch_for_test(name),
                "expected the Git Graph to know the newly created branch"
            )
        })
    })
}

/// A step that runs a write op which is expected to fail (checking out a branch
/// that doesn't exist), to exercise the error-banner rendering path.
pub fn run_failing_checkout() -> TestStep {
    TestStep::new("Run a write op that fails").with_action(|app, window_id, _| {
        let view = git_graph_view(app, window_id);
        view.update(app, |view, ctx| {
            view.handle_action(
                &GitGraphAction::RunOp(GitWriteOp::CheckoutBranch {
                    branch: "warp-no-such-branch-xyz".to_string(),
                    force: false,
                }),
                ctx,
            );
        });
    })
}

/// Asserts the op-error banner is showing — and, since the harness renders the
/// panel while polling this, that the banner renders without panicking.
pub fn assert_op_error_shown() -> AssertionCallback {
    Box::new(|app, window_id| {
        let view = git_graph_view(app, window_id);
        view.read(app, |view, _| {
            async_assert!(
                view.has_op_error_for_test(),
                "expected the failed write op to surface in the error banner"
            )
        })
    })
}
