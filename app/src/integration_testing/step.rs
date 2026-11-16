use warpui::async_assert;
use warpui::integration::{AssertionCallback, TestStep};

use super::terminal::assert_no_block_executing;
use crate::integration_testing::view_getters::terminal_view;

pub fn new_step_with_default_assertions(name: &str) -> TestStep {
    new_step_with_default_assertions_for_pane(name, 0, 0)
}

pub fn new_step_with_default_assertions_for_pane(
    name: &str,
    tab_index: usize,
    pane_index: usize,
) -> TestStep {
    // Add global assertions here
    TestStep::new(name)
        .add_named_assertion(
            "no pending model events",
            assert_no_pending_model_events_for_pane(tab_index, pane_index),
        )
        .add_named_assertion(
            "no block executing",
            assert_no_block_executing(tab_index, pane_index),
        )
}

pub fn assert_no_pending_model_events() -> AssertionCallback {
    assert_no_pending_model_events_for_pane(0, 0)
}

pub fn assert_no_pending_model_events_for_pane(
    tab_index: usize,
    pane_index: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
        terminal_view.read(app, |view, _ctx| {
            let model = view.model.lock();
            log::info!("events pending {}", model.are_any_events_pending());
            async_assert!(
                !model.are_any_events_pending(),
                "Should not be any pending model events",
            )
        })
    })
}
