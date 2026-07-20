use warpui::integration::{AssertionCallback, AssertionOutcome};
use warpui::{App, ViewHandle, WindowId, async_assert};

use crate::integration_testing::view_getters::{command_palette_view, workspace_view};
use crate::palette::PaletteMode;
use crate::pane_group::{PaneId, PaneView};
use crate::search::QueryFilter;
use crate::search::command_palette::ItemSummary;
use crate::terminal::TerminalView;

/// Used to determine which session should be the most recent in Navigation Palette integration tests.
pub enum RecentSession {
    First,
    Second,
}

/// Asserts that the navigation filter is currently enabled within the command palette.
pub fn assert_navigation_mode_enabled_in_command_palette() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let workspace = workspace_view(app, window_id);

        workspace.read(app, |workspace, ctx| {
            async_assert!(
                workspace.is_palette_mode_enabled(PaletteMode::Navigation, ctx),
                "Expected navigation palette to be enabled"
            )
        })
    })
}

/// Asserts that one of `first_pane_view` and `second_pane_view` is the most recent in the
/// command palette, depending on the value of [`RecentSession`].
pub fn check_recency(
    first_pane_view: ViewHandle<PaneView<TerminalView>>,
    second_pane_view: ViewHandle<PaneView<TerminalView>>,
    recency_test: RecentSession,
    app: &App,
    window_id: WindowId,
) -> AssertionOutcome {
    let command_palette = command_palette_view(app, window_id);
    command_palette.read(app, |palette, app| {
        assert_eq!(
            palette.active_query_filter(app),
            Some(QueryFilter::Sessions),
            "Sessions query filter is not applied"
        );

        let mut search_results = palette.search_results(app);
        let first_item = search_results
            .next()
            .expect("first item doesn't exist in search results")
            .accept_result()
            .to_summary();
        let second_item = search_results
            .next()
            .expect("second item doesn't exist in search results")
            .accept_result()
            .to_summary();

        let ItemSummary::Session {
            pane_view_locator: recent_session,
        } = first_item
        else {
            return AssertionOutcome::failure(
                "First item in command palette is not a session".to_string(),
            );
        };

        let ItemSummary::Session {
            pane_view_locator: previous_session,
        } = second_item
        else {
            return AssertionOutcome::failure(
                "second item in command palette is not a session".to_string(),
            );
        };

        match recency_test {
            RecentSession::First => {
                async_assert!(
                    recent_session.pane_id == PaneId::from_terminal_pane_view(&first_pane_view)
                        && previous_session.pane_id
                            == PaneId::from_terminal_pane_view(&second_pane_view),
                    "First session is not most recent., "
                )
            }
            RecentSession::Second => {
                async_assert!(
                    recent_session.pane_id == PaneId::from_terminal_pane_view(&second_pane_view)
                        && previous_session.pane_id
                            == PaneId::from_terminal_pane_view(&first_pane_view),
                    "Second session is not most recent."
                )
            }
        }
    })
}
