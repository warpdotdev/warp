use warpui::integration::AssertionCallback;
use warpui::{ViewHandle, async_assert, async_assert_eq};

use crate::context_chips::ContextChipKind;
use crate::context_chips::display_chip::DisplayChip;
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

/// Assertion on the raw detection token (e.g. `"rebase-interactive"`, `"bisect"`)
/// currently surfaced by the `GitOperationState` chip, or `None` if the chip has
/// no in-progress operation to report.
pub fn assert_git_operation_state_chip_value(
    tab_index: usize,
    expected_token: Option<&'static str>,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_index);
        terminal_view.read(app, |view, ctx| {
            let prompt = view.current_prompt();
            prompt.read(ctx, |prompt, ctx| {
                let value = prompt
                    .latest_chip_value(&ContextChipKind::GitOperationState, ctx)
                    .map(|value| value.to_string());
                async_assert_eq!(value.as_deref(), expected_token)
            })
        })
    })
}

/// Assertion that the `GitOperationState` chip's raw detection token is one of
/// `expected_tokens` (or `None`, when `expected_tokens` is empty).
///
/// Accepts a set of tokens because, for example, a plain `git rebase` may be
/// detected as either `rebase-interactive` or `rebase-apply` depending on the
/// installed git version's default backend (`rebase.backend`); either is a
/// correct detection of an in-progress rebase. Mirrors the equivalent
/// tolerance in `detect_finds_rebase_in_progress_from_a_linked_worktree`
/// (`git_operation_state_tests.rs`).
pub fn assert_git_operation_state_chip_value_one_of(
    tab_index: usize,
    expected_tokens: &'static [&'static str],
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_index);
        terminal_view.read(app, |view, ctx| {
            let prompt = view.current_prompt();
            prompt.read(ctx, |prompt, ctx| {
                let value = prompt
                    .latest_chip_value(&ContextChipKind::GitOperationState, ctx)
                    .map(|value| value.to_string());
                let matches = match value.as_deref() {
                    Some(token) => expected_tokens.contains(&token),
                    None => expected_tokens.is_empty(),
                };
                async_assert!(
                    matches,
                    "expected GitOperationState chip value to be one of {expected_tokens:?}, got {value:?}"
                )
            })
        })
    })
}

/// Assertion that the `GitOperationState` chip's dropdown menu is currently open.
pub fn assert_git_operation_state_menu_is_open(is_open: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let chips: Vec<ViewHandle<DisplayChip>> = app.views_of_type(window_id).unwrap_or_default();
        let actual = chips.iter().any(|chip| {
            chip.read(app, |chip, _ctx| {
                *chip.chip_kind() == ContextChipKind::GitOperationState
                    && chip.display_chip_kind().has_open_menu()
            })
        });
        async_assert_eq!(actual, is_open)
    })
}
