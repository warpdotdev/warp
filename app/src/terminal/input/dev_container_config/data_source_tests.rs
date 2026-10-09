use std::path::PathBuf;

use warpui::App;

use super::DevContainerConfigSelectorDataSource;
use crate::search::SyncDataSource;
use crate::search::data_source::Query;

/// `InlineMenuSelection::reset_to_best` highlights the last enabled row, so the
/// highest-precedence discovered config has to come out last on the empty-query path.
#[test]
fn empty_query_defaults_to_highest_precedence_config() {
    App::test((), |app| async move {
        let top_level = PathBuf::from("/repo/.devcontainer/devcontainer.json");
        let source = DevContainerConfigSelectorDataSource::new(vec![
            top_level.clone(),
            PathBuf::from("/repo/.devcontainer.json"),
            PathBuf::from("/repo/.devcontainer/backend/devcontainer.json"),
        ]);

        let results = app.read(|app| {
            source
                .run_query(&Query::from(""), app)
                .expect("empty query should succeed")
        });

        assert_eq!(
            results
                .last()
                .map(|result| result.accept_result().config_path),
            Some(top_level)
        );
    })
}

#[test]
fn empty_query_defaults_to_root_over_nested_when_top_level_is_absent() {
    App::test((), |app| async move {
        let root = PathBuf::from("/repo/.devcontainer.json");
        let source = DevContainerConfigSelectorDataSource::new(vec![
            root.clone(),
            PathBuf::from("/repo/.devcontainer/backend/devcontainer.json"),
        ]);

        let results = app.read(|app| {
            source
                .run_query(&Query::from(""), app)
                .expect("empty query should succeed")
        });

        assert_eq!(
            results
                .last()
                .map(|result| result.accept_result().config_path),
            Some(root)
        );
    })
}
