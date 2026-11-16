use warpui::async_assert;
use warpui::integration::AssertionCallback;

use crate::integration_testing::terminal::util::ExpectedOutput;
use crate::integration_testing::view_getters::pane_group_view;

/// Asserts that the tab has a pane at the given index with the expected title.
pub fn assert_pane_title(
    tab_index: usize,
    pane_index: usize,
    expected_title: impl ExpectedOutput + 'static,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let pane_group = pane_group_view(app, window_id, tab_index);
        pane_group.read(app, |pane_group, ctx| {
            match pane_group.pane_by_index(pane_index) {
                Some(pane) => {
                    let pane_title = pane.pane_configuration().as_ref(ctx).title().to_owned();
                    async_assert!(
                        expected_title.matches(&pane_title),
                        "Expected title for window_id={window_id}, tab_index={tab_index}, pane_index={pane_index} to be [{expected_title:?}], but got [{pane_title:?}]"
                    )
                },
                None => panic!("pane should exist for window_id={window_id}, tab_index={tab_index}, pane_index={pane_index}")
            }
        })
    })
}

/// Asserts that the active pane of the tab has an expected title.
pub fn assert_tab_title(
    tab_index: usize,
    expected_title: impl ExpectedOutput + 'static,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let pane_group = pane_group_view(app, window_id, tab_index);
        let title = pane_group.read(app, |pane_group, ctx| pane_group.display_title(ctx));
        async_assert!(
            expected_title.matches(&title),
            "Expected title of tab {tab_index} to match [{expected_title:?}], but was [{title}]"
        )
    })
}
