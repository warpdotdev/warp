use warpui::async_assert_eq;
use warpui::integration::{AssertionCallback, AssertionOutcome};

use crate::integration_testing::view_getters::pane_group_view;

pub fn assert_num_shared_sessions_in_pane_group(
    tab_index: usize,
    num_shared_sessions: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let pane_group = pane_group_view(app, window_id, tab_index);
        pane_group.read(app, |view, ctx| {
            async_assert_eq!(view.number_of_shared_sessions(ctx), num_shared_sessions)
        })
    })
}

pub fn assert_num_panes_in_tab(tab_index: usize, num_panes: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let pane_group = pane_group_view(app, window_id, tab_index);
        pane_group.read(app, |view, _| {
            async_assert_eq!(view.pane_count(), num_panes)
        })
    })
}

pub fn assert_focused_pane_index(tab_index: usize, pane_index: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let pane_group = pane_group_view(app, window_id, tab_index);
        pane_group.read(app, |view, ctx| {
            let Some(pane_id) = view.pane_id_from_index(pane_index) else {
                return AssertionOutcome::failure(format!("no pane at pane_index {pane_index}"));
            };
            async_assert_eq!(view.focused_pane_id(ctx), pane_id)
        })
    })
}

pub fn assert_pane_header_overlay_is_open(
    should_be_open: bool,
    tab_index: usize,
    pane_index: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        pane_group_view(app, window_id, tab_index).read(app, |pane_group, app| {
            pane_group
                .terminal_pane_view_at_pane_index(pane_index)
                .unwrap()
                .read(app, |pane_view, _| {
                    pane_view.header().read(app, |pane_header, _| {
                        async_assert_eq!(pane_header.is_overlay_open(), should_be_open)
                    })
                })
        })
    })
}
