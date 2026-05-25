use warpui::async_assert;
use warpui::integration::AssertionCallback;

use crate::context_chips::ContextChipKind;
use crate::integration_testing::view_getters::single_terminal_view_for_tab;

/// Assertion that the working dir chip is present in the current prompt.
pub fn assert_working_dir_is_present(tab_index: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_index);
        terminal_view.read(app, |view, ctx| {
            let prompt = view.current_prompt();
            prompt.read(ctx, |prompt, ctx| {
                async_assert!(
                    prompt
                        .latest_chip_value(&ContextChipKind::WorkingDirectory, ctx)
                        .is_some(),
                    "Working dir chip doesn't have a value"
                )
            })
        })
    })
}
