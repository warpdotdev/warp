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

fn any_directory_result(results: &[QueryResult<CommandPaletteItemAction>]) -> bool {
    results.iter().any(|r| {
        matches!(
            r.accept_result(),
            CommandPaletteItemAction::OpenDirectory { .. }
        )
    })
}

fn any_file_result(results: &[QueryResult<CommandPaletteItemAction>]) -> bool {
    results
        .iter()
        .any(|r| matches!(r.accept_result(), CommandPaletteItemAction::OpenFile { .. }))
}
fn current_folder_source(cached_contents: Vec<FileSearchResult>) -> FileDataSource {
    FileDataSource {
        mode: FileDataSourceMode::CurrentFolder { cached_contents },
    }
}
#[test]
fn test_files_palette_excludes_directories() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(FileSearchModel::new);
        app.add_singleton_model(|_| OpenedFilesModel::new());

        let contents = vec![dir("crates"), file("crates/foo.rs")];

        // Fuzzy-search (non-empty query) must exclude the matching directory
        // while still returning the matching file.
        let fuzzy_source = current_folder_source(contents.clone());
        let fuzzy_results = app
            .read(|ctx| {
                fuzzy_source.run_query(
                    &Query {
                        text: "crates".to_string(),
                        filters: HashSet::from([QueryFilter::Files]),
                    },
                    ctx,
                )
            })
            .await
            .expect("fuzzy query run failed");

        assert!(
            !any_directory_result(&fuzzy_results),
            "fuzzy-search results must not include directories"
        );
        assert!(
            any_file_result(&fuzzy_results),
            "expected the matching file to be included in fuzzy-search results"
        );

        // Zero-state (empty query) must also exclude the directory.
        let zero_source = current_folder_source(contents);
        let zero_results = app
            .read(|ctx| {
                zero_source.run_query(
                    &Query {
                        text: String::new(),
                        filters: HashSet::from([QueryFilter::Files]),
                    },
                    ctx,
                )
            })
            .await
            .expect("zero-state query run failed");

        assert!(
            !any_directory_result(&zero_results),
            "zero-state results must not include directories"
        );
        assert!(
            any_file_result(&zero_results),
            "expected the file to be included in zero-state results"
        );
    });
}
