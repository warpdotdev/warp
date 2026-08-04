use std::time::Duration;

use warp::integration_testing::assertions::{go_offline, go_online, join_a_workspace};
use warp::integration_testing::command_palette::{
    TestStepsExt, open_command_palette_and_run_action,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    execute_command_for_single_terminal_in_tab, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::window::save_active_window_id;
use warp::integration_testing::workflow::{
    assert_no_team_workflow_pane_open, assert_no_workflow_pane_open,
    assert_open_team_workflow_pane_count_equals, assert_open_workflow_pane_count_equals,
    assert_workflow_id, create_a_personal_workflow, open_workflow,
};
use warp::integration_testing::{self, view_of_type};
use warp::workflows::CategoriesView;
use warpui_core::integration::TestStep;
use warpui_core::{ViewHandle, async_assert_eq};

use super::{TEST_ONLY_ASSETS, new_builder};
use crate::Builder;

pub fn test_open_workflow_in_pane() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            create_a_personal_workflow("workflow_2_key")
                .add_assertion(save_active_window_id("first window")),
        )
        .with_step(
            open_workflow("first window", "workflow_2_key")
                .add_named_assertion_with_data_from_prior_step(
                    "Verify workflow is open",
                    assert_workflow_id(0, 0, "workflow_2_key"),
                ),
        )
}

pub fn test_create_personal_workflow_pane_from_command_palette() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(TestStep::new("Noop step").add_named_assertion(
            "Make sure no workflow panes are open",
            assert_no_workflow_pane_open(),
        ))
        .with_steps(
            open_command_palette_and_run_action("Create a New Personal Workflow")
                .add_named_assertion(
                    "There should be one workflow pane open",
                    assert_open_workflow_pane_count_equals(1),
                ),
        )
}

pub fn test_create_team_workflow_pane_from_command_palette() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(TestStep::new("Noop step").add_named_assertion(
            "Make sure no workflow panes are open",
            assert_no_workflow_pane_open(),
        ))
        .with_step(join_a_workspace())
        .with_step(go_offline())
        .with_step(
            TestStep::new("delay for test consistency")
                .set_post_step_pause(Duration::from_millis(250)),
        )
        .with_steps(
            open_command_palette_and_run_action("Create a New Team Workflow").add_named_assertion(
                "There should still not be any panes open",
                assert_no_team_workflow_pane_open(),
            ),
        )
        .with_step(go_online())
        .with_steps(
            open_command_palette_and_run_action("Create a New Team Workflow").add_named_assertion(
                "There should be an open workflow pane",
                assert_open_team_workflow_pane_count_equals(1),
            ),
        )
}

/// Adds a workflow file, containing two workflows, to a `.warp/workflows`
/// directory under a git repository and verifies that the workflows appear
/// in the workflow menu.
pub fn test_loading_project_workflows() -> Builder {
    new_builder()
        .with_setup(move |utils| {
            utils.set_env("WARP_CONFIG_WATCHER_DELAY_MS", Some((10).to_string()));
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Should have no project workflows").add_named_assertion(
                "Should have no project workflows",
                |app, window_id| {
                    let workflows: ViewHandle<CategoriesView> = view_of_type(app, window_id, 0);

                    workflows.read(app, |workflows, _| {
                        // Note that this can be a synchronous assertion because unlike the next assertion,
                        // we don't have concurrency with a WarpConfig watcher thread
                        async_assert_eq!(
                            workflows.project_workflows().count(),
                            0,
                            "There should not be any project workflows"
                        )
                    })
                },
            ),
        )
        // Create a git repository in the `repo` subdirectory.
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "git init repo && cd repo".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            TestStep::new("Write a new file containing two workflows").with_setup(|utils| {
                integration_testing::create_file_from_assets(
                    TEST_ONLY_ASSETS,
                    "test_workflow.yaml",
                    &utils
                        .test_dir()
                        .join("repo/.warp/workflows/test_workflow.yaml"),
                );
            }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Open the workflows browser to refresh the list of project workflows",
            )
            .with_keystrokes(&["ctrl-shift-R"]),
        )
        .with_step(
            TestStep::new("Verify the workflows were loaded successfully").add_named_assertion(
                "The two added workflows should be in the view",
                |app, window_id| {
                    let workflows: ViewHandle<CategoriesView> = view_of_type(app, window_id, 0);

                    let num_workflows =
                        workflows.read(app, |workflows, _| workflows.project_workflows().count());
                    async_assert_eq!(num_workflows, 2)
                },
            ),
        )
}
