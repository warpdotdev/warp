#![cfg(feature = "local_fs")]
use std::collections::HashSet;

use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::RepoMetadataModel;
use warpui::App;

use super::*;
use crate::code::opened_files::OpenedFilesModel;
use crate::search::data_source::Query;
use crate::search::files::model::FileSearchModel;
use crate::search::files::search_item::FileSearchResult;

fn file(path: &str) -> FileSearchResult {
    FileSearchResult {
        path: path.to_string(),
        project_directory: "/project".to_string(),
        is_directory: false,
    }
}

fn dir(path: &str) -> FileSearchResult {
    FileSearchResult {
        path: path.to_string(),
        project_directory: "/project".to_string(),
        is_directory: true,
    }
}

/// Regression test: a non-empty (fuzzy) files-palette query must include
/// matching directories, not just files. Previously the fuzzy path dropped all
/// directories, so a directory could never be found by typing a query even
/// though the zero-state (empty query) listing showed it — the inconsistency
/// reported by users.
#[test]
fn test_fuzzy_search_includes_matching_directories() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(FileSearchModel::new);
        app.add_singleton_model(|_| OpenedFilesModel::new());

        let data_source = FileDataSource::new_current_folder_with_contents(vec![
            dir("crates"),
            file("crates/foo.rs"),
        ]);

        let results = app
            .read(|ctx| {
                data_source.run_query(
                    &Query {
                        text: "crates".to_string(),
                        filters: HashSet::from([QueryFilter::Files]),
                    },
                    ctx,
                )
            })
            .await
            .expect("query run failed");

        // The matching directory must be present alongside the matching file.
        assert!(
            results.iter().any(|r| matches!(
                r.accept_result(),
                CommandPaletteItemAction::OpenDirectory { .. }
            )),
            "expected the matching directory to be included in filtered results"
        );
        assert!(
            results
                .iter()
                .any(|r| matches!(r.accept_result(), CommandPaletteItemAction::OpenFile { .. })),
            "expected the matching file to be included in filtered results"
        );
    });
}
