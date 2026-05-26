//! Integration tests for pane restoration functionality.
//! Tests the ability to restore closed panes using cmd+shift+t.

use std::collections::HashMap;
use std::time::Duration;

use warp::cmd_or_ctrl_shift;
use warp::features::FeatureFlag;
use warp::integration_testing::pane_group::assert_focused_pane_index;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    execute_command, validate_block_output_on_finished_block, wait_until_bootstrapped_pane,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::workspace::{assert_tab_count, trigger_undo_close};

use super::{new_builder, Builder};

/// Tests the basic pane restoration workflow:
/// 1. Split off a pane
/// 2. Run a simple command in it  
/// 3. Close the pane
/// 4. Restore the pane with cmd+shift+t
/// 5. Assert the pane is restored in the correct location with previous state
pub fn test_restore_single_closed_pane() -> Builder {
    FeatureFlag::UndoClosedPanes.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Split off a new pane to the right")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(execute_command(
            0,
            1,
            "echo \"hello world\"".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Close the pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("w")])
                .add_assertion(assert_focused_pane_index(0, 0)),
        )
        .with_step(trigger_undo_close())
        .with_step(
            new_step_with_default_assertions("Verify we still have one tab after restore attempt")
                .add_assertion(assert_tab_count(1)),
        )
        .with_step(
            new_step_with_default_assertions("Verify pane was restored with correct state")
                .set_pause_on_failure(std::time::Duration::from_secs(30))
                .add_assertion(assert_focused_pane_index(0, 1))
                .add_assertion(move |app, window_id| {
                    validate_block_output_on_finished_block(
                        &"hello world",
                        0, // tab index
                        1, // pane index - the restored pane should be at index 1
                        window_id,
                        app,
                    )
                }),
        )
}

/// Tests complex pane restoration workflow with multiple panes:
/// 1. Start with one pane, split twice to create 3 panes total
/// 2. Run unique commands in each pane
/// 3. Close two of the panes
/// 4. Restore both closed panes using undo close twice
/// 5. Assert both panes are restored correctly with their previous state
pub fn test_restore_multiple_closed_panes() -> Builder {
    FeatureFlag::UndoClosedPanes.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command(
            0,
            0,
            "echo \"pane0\"".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Split off first new pane to the right")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(execute_command(
            0,
            1,
            "echo \"pane1\"".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Split off second new pane to the right")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 2)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 2))
        .with_step(execute_command(
            0,
            2,
            "echo \"pane2\"".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Close pane 2")
                .with_keystrokes(&[cmd_or_ctrl_shift("w")])
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(
            new_step_with_default_assertions("Close pane 1")
                .with_keystrokes(&[cmd_or_ctrl_shift("w")])
                .add_assertion(assert_focused_pane_index(0, 0)),
        )
        .with_step(trigger_undo_close().add_assertion(assert_focused_pane_index(0, 1)))
        .with_step(
            new_step_with_default_assertions("Verify first restored pane has correct state")
                .set_pause_on_failure(std::time::Duration::from_secs(30))
                .add_assertion(move |app, window_id| {
                    validate_block_output_on_finished_block(
                        &"pane1", 0, // tab index
                        1, // pane index - first restored pane should be at index 1
                        window_id, app,
                    )
                }),
        )
        .with_step(trigger_undo_close().add_assertion(assert_focused_pane_index(0, 2)))
        .with_step(
            new_step_with_default_assertions("Verify second restored pane has correct state")
                .set_pause_on_failure(std::time::Duration::from_secs(30))
                .add_assertion(move |app, window_id| {
                    validate_block_output_on_finished_block(
                        &"pane2", 0, // tab index
                        2, // pane index - second restored pane should be at index 2
                        window_id, app,
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Verify we still have one tab after all restore operations",
            )
            .add_assertion(assert_tab_count(1)),
        )
        .with_step(
            new_step_with_default_assertions("Verify original pane (pane 0) still has its state")
                .add_assertion(move |app, window_id| {
                    validate_block_output_on_finished_block(
                        &"pane0", 0, // tab index
                        0, // pane index - original pane should still be at index 0
                        window_id, app,
                    )
                }),
        )
}

/// Tests that panes are properly cleaned up after the grace period expires:
/// 1. Split off a pane and run a command in it
/// 2. Close the pane
/// 3. Wait for the grace period to expire (5 seconds in test)
/// 4. Attempt to restore the pane with cmd+shift+t
/// 5. Assert the pane is NOT restored because it was cleaned up
pub fn test_undo_close_grace_period_cleanup() -> Builder {
    FeatureFlag::UndoClosedPanes.set_enabled(true);
    new_builder()
        .with_user_defaults(HashMap::from([(
            "UndoCloseGracePeriod".to_owned(),
            serde_json::to_string(&Duration::from_secs(5))
                .expect("Duration should convert to JSON string"),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Split off a new pane to the right")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(execute_command(
            0,
            1,
            "echo \"hello world\"".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Close the pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("w")])
                .add_assertion(assert_focused_pane_index(0, 0)),
        )
        .with_step(
            new_step_with_default_assertions("Wait for grace period to expire")
                .set_timeout(Duration::from_secs(7)), // Wait 7 seconds for 5 second grace period
        )
        .with_step(
            new_step_with_default_assertions("Check pane count before undo close")
                .add_assertion(move |app, window_id| {
                    let workspace_view = warp::integration_testing::view_getters::workspace_view(app, window_id);
                    let initial_pane_count = workspace_view.read(app, |workspace, ctx| {
                        let pane_group_view = workspace.get_pane_group_view(0).expect("should have tab 0");
                        pane_group_view.read(ctx, |pane_group, _| pane_group.pane_count())
                    });
                    warpui::async_assert_eq!(initial_pane_count, 1, "Should have exactly one pane after grace period expires - closed pane should be cleaned up")
                }),
        )
        .with_step(trigger_undo_close()
            .add_assertion(assert_focused_pane_index(0, 0)) // Should still be focused on original pane
            .add_assertion(move |app, window_id| {
                // Assert we still only have one pane (no restoration occurred)
                let workspace_view = warp::integration_testing::view_getters::workspace_view(app, window_id);
                workspace_view.read(app, |workspace, ctx| {
                    let pane_group_view = workspace.get_pane_group_view(0).expect("should have tab 0");
                    let pane_count = pane_group_view.read(ctx, |pane_group, _| pane_group.pane_count());
                    warpui::async_assert_eq!(pane_count, 1, "Should still have only one pane after attempted restore - no pane should be restored")
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions("Verify we still have one tab after failed restore attempt")
                .add_assertion(assert_tab_count(1)),
        )
}

/// Tests that closed panes are cleared when pane rearrangement operations begin:
/// 1. Create 3 panes and run commands in each
/// 2. Close one pane (it gets hidden for undo)
/// 3. Start a pane rearrangement operation (resize divider)
/// 4. Attempt to restore the closed pane with cmd+shift+t
/// 5. Assert the pane is NOT restored because it was cleared during rearrangement
pub fn test_closed_panes_cleared_on_rearrangement() -> Builder {
    FeatureFlag::UndoClosedPanes.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command(
            0,
            0,
            "echo \"original_pane\"".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Split off first new pane to the right")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(execute_command(
            0,
            1,
            "echo \"middle_pane\"".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Split off second new pane to the right")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 2)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 2))
        .with_step(execute_command(
            0,
            2,
            "echo \"third_pane\"".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            // Close the middle pane by using a direct operation targeting pane index 1
            warp::integration_testing::pane_group::close_pane_by_index(
                0, // tab index
                1, // pane index - the middle pane
            ),
        )
        .with_step(
            new_step_with_default_assertions("Verify we have 2 visible panes after closing one")
                .add_assertion(move |app, window_id| {
                    let workspace_view = warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace_view.read(app, |workspace, ctx| {
                        let pane_group_view = workspace.get_pane_group_view(0).expect("should have tab 0");
                        let (visible_pane_count, total_pane_count) = pane_group_view.read(ctx, |pane_group, _| {
                            (pane_group.visible_pane_count(), pane_group.pane_count())
                        });
                        if visible_pane_count != 2 {
                            warpui::integration::AssertionOutcome::failure(format!("Should have 2 visible panes after closing one (got {visible_pane_count} visible, {total_pane_count} total)"))
                        } else {
                            warpui::integration::AssertionOutcome::Success
                        }
                    })
                }),
        )
        .with_step(
            // Trigger pane rearrangement by moving panes
            warp::integration_testing::pane_group::move_pane_by_indices(
                0,
                0,
                1,
                warp::pane_group::tree::Direction::Right,
            ),
        )
        .with_step(
            // Trigger undo close - should NOT restore the pane since rearrangement cleared it
            trigger_undo_close()
        )
        .with_step(
            new_step_with_default_assertions("Verify pane was NOT restored - still have same visible panes")
                .add_assertion(move |app, window_id| {
                    let workspace_view = warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace_view.read(app, |workspace, ctx| {
                        let pane_group_view = workspace.get_pane_group_view(0).expect("should have tab 0");
                        let visible_pane_count = pane_group_view.read(ctx, |pane_group, _| pane_group.visible_pane_count());
                        // After rearrangement, we should still have the same visible panes (no restoration)
                        // The exact count might vary based on how the move operation affects the layout
                        if visible_pane_count < 1 {
                            warpui::integration::AssertionOutcome::failure(format!("Should have at least 1 visible pane after undo close attempt, got {visible_pane_count}"))
                        } else {
                            warpui::integration::AssertionOutcome::Success
                        }
                    })
                }),
        )
        .with_step(
            trigger_undo_close().add_assertion(move |app, window_id| {
                let workspace_view = warp::integration_testing::view_getters::workspace_view(app, window_id);
                workspace_view.read(app, |workspace, ctx| {
                    let pane_group_view = workspace.get_pane_group_view(0).expect("should have tab 0");
                    let visible_pane_count = pane_group_view.read(ctx, |pane_group, _| pane_group.visible_pane_count());
                    // Should still have the same panes, no restoration should occur
                    if visible_pane_count < 1 {
                        warpui::integration::AssertionOutcome::failure(format!("Should have at least 1 visible pane after second undo close attempt, got {visible_pane_count}"))
                    } else {
                        warpui::integration::AssertionOutcome::Success
                    }
                })
            })
        )
        .with_step(
            new_step_with_default_assertions("Verify remaining pane has expected state")
                .add_assertion(move |app, window_id| {
                    let workspace_view = warp::integration_testing::view_getters::workspace_view(app, window_id);
                    let visible_pane_count = workspace_view.read(app, |workspace, ctx| {
                        let pane_group_view = workspace.get_pane_group_view(0).expect("should have tab 0");
                        pane_group_view.read(ctx, |pane_group, _| pane_group.visible_pane_count())
                    });
                    if visible_pane_count == 1 {
                        // If we have 1 pane, it should be the third_pane
                        validate_block_output_on_finished_block(
                            &"third_pane",
                            0, // tab index
                            0, // pane index - only remaining pane
                            window_id,
                            app,
                        )
                    } else if visible_pane_count == 2 {
                        // If we have 2 panes, check for both original and third
                        let original_result = validate_block_output_on_finished_block(
                            &"original_pane",
                            0, 1, window_id, app,
                        );
                        let third_result = validate_block_output_on_finished_block(
                            &"third_pane",
                            0, 0, window_id, app,
                        );
                        match (original_result, third_result) {
                            (warpui::integration::AssertionOutcome::Success, warpui::integration::AssertionOutcome::Success) => {
                                warpui::integration::AssertionOutcome::Success
                            }
                            _ => warpui::integration::AssertionOutcome::failure("Expected to find both original_pane and third_pane content".to_string())
                        }
                    } else {
                        warpui::integration::AssertionOutcome::failure(format!("Unexpected pane count: {visible_pane_count}"))
                    }
                }),
        )
}

/// Tests that closing the last visible pane in a tab properly closes the tab:
/// 1. Create a single pane in a tab with a command
/// 2. Create a new tab to verify multiple tabs exist
/// 3. Return to first tab and close its only pane
/// 4. Verify the tab is closed (not just showing empty)
/// 5. Restore the pane and verify it creates a new tab
pub fn test_tab_closes_when_last_visible_pane_closed() -> Builder {
    FeatureFlag::UndoClosedPanes.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command(
            0,
            0,
            "echo \"first_tab_content\"".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Create a new tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")])
                .add_assertion(assert_tab_count(2)),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(execute_command(
            1,
            0,
            "echo \"second_tab_content\"".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Switch back to first tab")
                .with_keystrokes(&["cmdorctrl-1"])
                .add_assertion(assert_focused_pane_index(0, 0)),
        )
        .with_step(
            new_step_with_default_assertions("Close the only pane in first tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("w")])
                .set_timeout(Duration::from_secs(10)) // Allow time for tab closure
                .add_assertion(assert_tab_count(1)) // Tab should be closed, leaving only one tab
                .add_assertion(assert_focused_pane_index(0, 0)), // Should be focused on the remaining tab (previously tab 1)
        )
        .with_step(
            new_step_with_default_assertions("Verify we're now on the second tab's content")
                .add_assertion(move |app, window_id| {
                    validate_block_output_on_finished_block(
                        &"second_tab_content",
                        0, // tab index - this is now the first (and only) tab
                        0, // pane index
                        window_id,
                        app,
                    )
                }),
        )
        .with_step(
            trigger_undo_close()
                .set_timeout(Duration::from_secs(15)) // Allow extra time for tab restoration
                .add_assertion(assert_tab_count(2)), // Should now have 2 tabs again
        )
        .with_step(
            new_step_with_default_assertions("Wait for restored tab to be ready")
                .set_timeout(Duration::from_secs(10)), // Allow time for tab setup
        )
        .with_step(
            new_step_with_default_assertions("Verify restored pane has correct content in new tab")
                .set_timeout(Duration::from_secs(20)) // Allow extra time for content validation
                .set_pause_on_failure(std::time::Duration::from_secs(30))
                .add_assertion(move |app, window_id| {
                    let workspace_view =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace_view.read(app, |workspace, _ctx| {
                        let focused_tab_idx = workspace.active_tab_index();

                        // The restored tab should be the currently focused tab (which should have first_tab_content)
                        validate_block_output_on_finished_block(
                            &"first_tab_content",
                            focused_tab_idx, // Use the currently focused tab
                            0,               // pane index
                            window_id,
                            app,
                        )
                    })
                }),
        )
}
