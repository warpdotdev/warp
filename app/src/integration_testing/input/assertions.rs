use warpui::integration::AssertionCallback;
use warpui::{async_assert, async_assert_eq};

use crate::integration_testing::view_getters::{input_view, single_input_view_for_tab};
use crate::terminal::input::InputSuggestionsMode;

pub fn assert_workflow_info_box_is_open(tab_idx: usize, pane_idx: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let input = input_view(app, window_id, tab_idx, pane_idx);
        input.read(app, |input, _ctx| {
            async_assert!(input.is_workflows_info_box_open())
        })
    })
}

pub fn input_editor_is_focused(tab_idx: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let input = single_input_view_for_tab(app, window_id, tab_idx);
        input.read(app, |input, ctx| {
            async_assert!(
                input.editor().is_focused(ctx),
                "Input editor should be focused"
            )
        })
    })
}

pub fn input_editor_is_not_focused(tab_idx: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let input = single_input_view_for_tab(app, window_id, tab_idx);
        input.read(app, |input, ctx| {
            async_assert!(
                !input.editor().is_focused(ctx),
                "Input editor should not be focused"
            )
        })
    })
}

pub fn input_contains_string(tab_idx: usize, string: String) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let input = single_input_view_for_tab(app, window_id, tab_idx);
        input.read(app, |view, ctx| {
            async_assert_eq!(
                view.buffer_text(ctx),
                string,
                "Input should contain string {string}"
            )
        })
    })
}

pub fn input_is_empty(tab_idx: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let input = single_input_view_for_tab(app, window_id, tab_idx);
        input.read(app, |view, ctx| {
            async_assert!(view.buffer_text(ctx).is_empty(), "Input should be empty")
        })
    })
}

pub fn tab_completions_menu_is_open(tab_idx: usize, is_opened: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let input = single_input_view_for_tab(app, window_id, tab_idx);
        input.read(app, |view, ctx| {
            let assertion = if is_opened {
                matches!(
                    view.suggestions_mode_model().as_ref(ctx).mode(),
                    InputSuggestionsMode::CompletionSuggestions { .. }
                )
            } else {
                matches!(
                    view.suggestions_mode_model().as_ref(ctx).mode(),
                    InputSuggestionsMode::Closed
                )
            };

            async_assert!(assertion)
        })
    })
}

pub fn latest_buffer_operations_are_empty(
    tab_idx: usize,
    should_be_empty: bool,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let input = single_input_view_for_tab(app, window_id, tab_idx);
        input.read(app, |view, _ctx| {
            if should_be_empty {
                async_assert!(view.latest_buffer_operations().count() == 0)
            } else {
                async_assert!(view.latest_buffer_operations().count() > 0)
            }
        })
    })
}

#[derive(Clone)]
pub enum AutosuggestionState {
    /// The autosuggestion is inactive.
    Closed,
    /// The autosuggestion is active with _some_ text.
    Active,
    /// The autosuggestion is active and is specifically some text.
    ActiveWithText(String),
}

pub fn assert_autosuggestion_state(
    tab_idx: usize,
    state: AutosuggestionState,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let input = single_input_view_for_tab(app, window_id, tab_idx);
        let state = state.clone();
        input.read(app, move |view, ctx| {
            let autosuggestion = view.editor().as_ref(ctx).current_autosuggestion_text();
            let assertion = match state {
                AutosuggestionState::Closed => autosuggestion.is_none(),
                AutosuggestionState::Active => autosuggestion.is_some(),
                AutosuggestionState::ActiveWithText(expected) => {
                    autosuggestion.is_some_and(|s| expected.as_str() == s)
                }
            };
            async_assert!(assertion)
        })
    })
}
