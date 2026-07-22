use remote_server::codebase_index_proto::{RemoteCodebaseIndexState, RemoteCodebaseIndexStatus};
use warpui::App;

use super::super::settings_page::{FilteredPageType, MatchData, PageType, SettingsWidget};
#[cfg(feature = "local_fs")]
use super::ExternalEditorCodeWidget;
use super::{
    AutoOpenCodeReviewPaneCodeWidget, AutoSaveToggleWidget, CodeReviewDiffStatsToggleWidget,
    CodeReviewPanelToggleWidget, CodeSettingsPageView, CodeSubpage, CodeSubpageHeaderWidget,
    FormatOnSaveToggleWidget, GlobalSearchToggleWidget, ProjectExplorerToggleWidget,
    ShowHiddenFilesToggleWidget, remote_codebase_index_limit_reached,
};

fn remote_status_with_failure(failure_message: Option<&str>) -> RemoteCodebaseIndexStatus {
    RemoteCodebaseIndexStatus {
        repo_path: "/workspaces/repo".to_string(),
        state: RemoteCodebaseIndexState::Unavailable,
        last_updated_epoch_millis: Some(1),
        progress_completed: None,
        progress_total: None,
        failure_message: failure_message.map(ToOwned::to_owned),
        root_hash: None,
    }
}

#[test]
fn remote_index_limit_failure_is_detected_from_status_message() {
    let status = remote_status_with_failure(Some(
        "Cannot index remote codebase because the maximum number of codebase indexes has been reached.",
    ));

    assert!(remote_codebase_index_limit_reached(&status));
}

#[test]
fn other_unavailable_failures_are_not_index_limit_failures() {
    let status = remote_status_with_failure(Some(
        "Cannot index remote codebase because indexing did not start.",
    ));

    assert!(!remote_codebase_index_limit_reached(&status));
}

// ── Subpage search rebuild/restore cycle ─────────────────────────────────────
// These tests drive the real PageType state cycle that regressed in APP-4910:
// `set_active_subpage` rebuilds the active subpage by constructing a fresh
// `PageType::new_uncategorized`, whose filter starts containing every widget
// index (every widget visible). Without reapplying the active query the rendered
// content area therefore shows every widget even though the sidebar match count
// is narrowed. `SettingsView::restore_active_subpage_filter` fixes this by calling
// `update_filter` after every rebuild. The tests below rebuild via
// `PageType::new_uncategorized` (exactly what `set_active_subpage` does), reapply
// via `update_filter`, and assert the rendered widget indices through
// `get_filtered()` — the same view the settings content area renders.

/// Build the Editor and Code Review subpage widgets exactly as
/// `CodeSettingsPageView::set_active_subpage(Some(EditorAndCodeReview))` does, so
/// the regression exercises the same widget set the production rebuild constructs.
fn editor_and_code_review_widgets() -> Vec<Box<dyn SettingsWidget<View = CodeSettingsPageView>>> {
    let mut widgets: Vec<Box<dyn SettingsWidget<View = CodeSettingsPageView>>> =
        vec![Box::new(CodeSubpageHeaderWidget {
            title: CodeSubpage::EditorAndCodeReview.title(),
        })];
    #[cfg(feature = "local_fs")]
    widgets.push(Box::new(ExternalEditorCodeWidget));
    widgets.extend([
        Box::new(AutoOpenCodeReviewPaneCodeWidget::default())
            as Box<dyn SettingsWidget<View = CodeSettingsPageView>>,
        Box::new(CodeReviewPanelToggleWidget::default()),
        Box::new(CodeReviewDiffStatsToggleWidget::default()),
        Box::new(ProjectExplorerToggleWidget::default()),
        Box::new(GlobalSearchToggleWidget::default()),
        Box::new(ShowHiddenFilesToggleWidget::default()),
        Box::new(FormatOnSaveToggleWidget::default()),
        Box::new(AutoSaveToggleWidget::default()),
    ]);
    widgets
}

/// Read the rendered widget search terms out of an Uncategorized `PageType`, in the
/// order `get_filtered()` yields them. This is the same view the settings content
/// area renders, so assertions here check what the user actually sees.
fn filtered_uncategorized_search_terms(page: &PageType<CodeSettingsPageView>) -> Vec<&str> {
    match page.get_filtered() {
        FilteredPageType::Uncategorized { widgets, .. } => {
            widgets.iter().map(|widget| widget.search_terms()).collect()
        }
        _ => panic!("expected an Uncategorized page after a subpage rebuild"),
    }
}

/// `update_filter` returns a `MatchData` count for non-empty queries; extract it.
fn match_count(match_data: MatchData) -> usize {
    match match_data {
        MatchData::Countable(n) => n,
        MatchData::Uncounted(true) => 1,
        MatchData::Uncounted(false) => 0,
    }
}

#[test]
fn code_subpage_search_reapplies_filter_after_restore() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            // `set_active_subpage` rebuilds the active subpage by constructing a fresh
            // `PageType::new_uncategorized`, whose filter starts containing every widget
            // index (every widget visible). This is the rebuild that runs after the
            // page-level search filter pass in handle_search_editor_event.
            let widgets = editor_and_code_review_widgets();
            let total = widgets.len();
            let mut page = PageType::new_uncategorized(widgets, None);

            // Regression signature: right after the rebuild, before the query is
            // reapplied, get_filtered() returns every widget. This is the stale,
            // all-visible state SettingsView::restore_active_subpage_filter exists to
            // correct.
            assert_eq!(
                filtered_uncategorized_search_terms(&page).len(),
                total,
                "a freshly rebuilt subpage must start with every widget visible"
            );

            // The fix reapplies the active query via update_filter after the rebuild.
            let matches = page.update_filter("auto save", ctx);
            assert_eq!(match_count(matches), 1);

            // Only the Auto save widget remains; no stale widgets are visible.
            let restored = filtered_uncategorized_search_terms(&page);
            assert_eq!(restored.len(), 1);
            assert_eq!(
                restored[0],
                AutoSaveToggleWidget::default().search_terms(),
                "only the Auto save widget should match \"auto save\""
            );
        });
    });
}

#[test]
fn restored_auto_selected_subpage_reapplies_filter() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            // The user is on the Codebase Indexing subpage, which has no "auto save"
            // widget. Search computes zero matches there, so the handler auto-selects
            // the Editor and Code Review subpage as the new destination.
            let indexing_widgets: Vec<Box<dyn SettingsWidget<View = CodeSettingsPageView>>> =
                vec![Box::new(CodeSubpageHeaderWidget {
                    title: CodeSubpage::Indexing.title(),
                })];
            let mut indexing_page = PageType::new_uncategorized(indexing_widgets, None);
            assert_eq!(
                match_count(indexing_page.update_filter("auto save", ctx)),
                0,
                "Codebase Indexing has no auto-save widget"
            );
            assert!(filtered_uncategorized_search_terms(&indexing_page).is_empty());

            // Auto-selection rebuilds the Editor and Code Review destination. Right
            // after the rebuild it shows every widget (stale); the fix reapplies the
            // query so only the matching widget is visible.
            let editor_widgets = editor_and_code_review_widgets();
            let total = editor_widgets.len();
            let mut editor_page = PageType::new_uncategorized(editor_widgets, None);
            assert_eq!(
                filtered_uncategorized_search_terms(&editor_page).len(),
                total,
                "rebuilt destination starts all-visible before the query is reapplied"
            );
            editor_page.update_filter("auto save", ctx);
            let restored = filtered_uncategorized_search_terms(&editor_page);
            assert_eq!(restored.len(), 1);
            assert_eq!(restored[0], AutoSaveToggleWidget::default().search_terms());
        });
    });
}

#[test]
fn clearing_search_restores_all_subpage_widgets() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let widgets = editor_and_code_review_widgets();
            let total = widgets.len();
            let mut page = PageType::new_uncategorized(widgets, None);

            // A query narrows the rebuilt subpage to the single matching widget.
            page.update_filter("auto save", ctx);
            assert_eq!(filtered_uncategorized_search_terms(&page).len(), 1);

            // Clearing the query restores every widget, matching the no-search state.
            page.update_filter("", ctx);
            assert_eq!(
                filtered_uncategorized_search_terms(&page).len(),
                total,
                "clearing the search query must restore every subpage widget"
            );
        });
    });
}
