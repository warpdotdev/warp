use warpui::async_assert;
use warpui::integration::AssertionCallback;

use crate::integration_testing::view_getters::{command_palette_view, workspace_view};

/// Asserts that the command palette is currently open.
pub fn assert_command_palette_is_open() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let workspace = workspace_view(app, window_id);

        workspace.read(app, |workspace, _| {
            async_assert!(
                workspace.is_palette_open(),
                "Expected palette to be open, but it was closed"
            )
        })
    })
}

/// Asserts that the command palette is currently closed.
pub fn assert_command_palette_is_closed() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let workspace = workspace_view(app, window_id);

        workspace.read(app, |workspace, _| {
            async_assert!(
                !workspace.is_palette_open(),
                "Expected palette to be closed, but it was open"
            )
        })
    })
}

/// Asserts that the command palette currently has at least one search result.
pub fn assert_command_palette_has_results() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let palette = command_palette_view(app, window_id);

        palette.read(app, |palette, ctx| {
            async_assert!(
                palette.search_results(ctx).next().is_some(),
                "Expected command palette to have results, but it was empty"
            )
        })
    })
}
