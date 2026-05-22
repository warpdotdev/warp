use warpui::integration::AssertionCallback;
use warpui::{async_assert, async_assert_eq};

use crate::integration_testing::view_getters::{command_search_view, workspace_view};
use crate::search::QueryFilter;

pub fn assert_command_search_is_open() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let workspace_view = workspace_view(app, window_id);
        workspace_view.read(app, |workspace, _ctx| {
            async_assert!(workspace.is_command_search_open())
        })
    })
}

pub fn assert_history_filter_is_active() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let command_search_view = command_search_view(app, window_id);
        command_search_view.read(app, |command_search_view, ctx| {
            let search_bar = command_search_view.search_bar();
            async_assert_eq!(
                search_bar.as_ref(ctx).active_query_filter(ctx),
                Some(QueryFilter::History)
            )
        })
    })
}

pub fn assert_command_search_has_results() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let command_search_view = command_search_view(app, window_id);
        command_search_view.read(app, |command_search_view, ctx| {
            async_assert!(
                command_search_view.has_search_results(ctx),
                "Expected command search to have results, but it was empty"
            )
        })
    })
}

pub fn assert_query(query: impl AsRef<str> + 'static) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let command_search_view = command_search_view(app, window_id);
        command_search_view.read(app, |command_search_view, ctx| {
            let search_bar = command_search_view.search_bar();
            async_assert_eq!(search_bar.as_ref(ctx).query(ctx).as_str(), query.as_ref())
        })
    })
}
