use fuzzy_match::FuzzyMatchResult;
use repo_metadata::RepoMetadataModel;
use repo_metadata::repositories::DetectedRepositories;
use warpui::{App, SingletonEntity};

use super::super::search_item::{FileSearchItem, FileSearchResult};
use super::FileSearchModel;

#[cfg(test)]
mod file_search_model_tests {
    use super::*;

    #[test]
    fn test_file_search_model_creation() {
        App::test((), |app| async move {
            app.add_singleton_model(|_| DetectedRepositories::default());
            app.add_singleton_model(RepoMetadataModel::new);
            app.add_singleton_model(FileSearchModel::new);
            // Verify the singleton was registered and is accessible
            app.read(|app| {
                let _model = FileSearchModel::as_ref(app);
            });
        });
    }

    #[cfg(feature = "local_fs")]
    #[test]
    fn file_tree_completion_invalidates_cache_repopulated_during_update() {
        use std::sync::Arc;

        use repo_metadata::file_tree_store::FileTreeState;
        use repo_metadata::{
            DirectoryEntry, Entry, FileMetadata, MetadataUpdateType, RepoMetadataEvent,
            RepositoryIdentifier,
        };
        use tempfile::tempdir;
        use warp_util::standardized_path::StandardizedPath;
        use warpui::elements::Empty;
        use warpui::platform::WindowStyle;
        use warpui::windowing::WindowManager;
        use warpui::{AppContext, Element, Entity, TypedActionView, View};

        use crate::terminal::model::session::Session;
        use crate::workspace::ActiveSession;

        struct TestView;

        impl Entity for TestView {
            type Event = ();
        }

        impl View for TestView {
            fn ui_name() -> &'static str {
                "FileSearchModelTestView"
            }

            fn render(&self, _app: &AppContext) -> Box<dyn Element> {
                Empty::new().finish()
            }
        }

        impl TypedActionView for TestView {
            type Action = ();
        }

        fn state_with_file(root: &StandardizedPath, filename: &str) -> FileTreeState {
            FileTreeState::new(
                Entry::Directory(DirectoryEntry {
                    path: root.clone(),
                    children: vec![Entry::File(FileMetadata::new(
                        root.to_local_path_lossy().join(filename),
                        false,
                    ))],
                    ignored: false,
                    loaded: true,
                }),
                Vec::new(),
                None,
            )
        }

        App::test((), |mut app| async move {
            app.add_singleton_model(|_| DetectedRepositories::default());
            app.add_singleton_model(RepoMetadataModel::new);
            app.add_singleton_model(FileSearchModel::new);
            app.add_singleton_model(|_| ActiveSession::default());

            let temp = tempdir().expect("temporary repository should be created");
            let repo_path = dunce::canonicalize(temp.path()).unwrap();
            let repo = StandardizedPath::from_local_canonicalized(&repo_path).unwrap();
            let repo_id = RepositoryIdentifier::local(repo.clone());
            let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, |_ctx| TestView);
            WindowManager::handle(&app).update(&mut app, |windowing_state, _ctx| {
                windowing_state.overwrite_for_test(windowing_state.stage(), Some(window_id));
            });
            ActiveSession::handle(&app).update(&mut app, |active_session, ctx| {
                active_session.set_session_for_test(
                    window_id,
                    Arc::new(Session::test()),
                    Some(repo_path),
                    None,
                    ctx,
                );
            });
            DetectedRepositories::handle(&app).update(&mut app, |repositories, _ctx| {
                repositories.insert_test_repo_root(repo.clone());
            });
            RepoMetadataModel::handle(&app).update(&mut app, |model, ctx| {
                model.insert_test_state(
                    repo.clone(),
                    state_with_file(&repo, "before-update.txt"),
                    ctx,
                );
            });

            let initial = app.read(|ctx| FileSearchModel::as_ref(ctx).get_repo_contents("", ctx));
            assert!(
                initial
                    .iter()
                    .any(|result| result.path == "before-update.txt")
            );

            RepoMetadataModel::handle(&app).update(&mut app, |_, ctx| {
                ctx.emit(RepoMetadataEvent::FileTreeUpdated {
                    ids: vec![repo_id.clone()],
                });
            });
            let stale = app.read(|ctx| FileSearchModel::as_ref(ctx).get_repo_contents("", ctx));
            assert!(
                stale
                    .iter()
                    .any(|result| result.path == "before-update.txt")
            );

            RepoMetadataModel::handle(&app).update(&mut app, |model, ctx| {
                model.insert_test_state(
                    repo.clone(),
                    state_with_file(&repo, "after-update.txt"),
                    ctx,
                );
                ctx.emit(RepoMetadataEvent::FileTreeEntryUpdated {
                    id: repo_id,
                    update_type: MetadataUpdateType::FullReplace,
                });
            });
            let current = app.read(|ctx| FileSearchModel::as_ref(ctx).get_repo_contents("", ctx));
            assert!(
                current
                    .iter()
                    .any(|result| result.path == "after-update.txt")
            );
            assert!(
                current
                    .iter()
                    .all(|result| result.path != "before-update.txt")
            );
        });
    }
    #[test]
    fn test_fuzzy_match_path_empty_query() {
        let result = FileSearchModel::fuzzy_match_path("src/main.rs", "");

        assert!(result.is_some());
        let match_result = result.unwrap();
        assert_eq!(match_result.score, 0);
        assert!(match_result.matched_indices.is_empty());
    }

    #[test]
    fn test_fuzzy_match_path_whitespace_only_query() {
        let result = FileSearchModel::fuzzy_match_path("src/main.rs", "   ");

        assert!(result.is_some());
        let match_result = result.unwrap();
        assert_eq!(match_result.score, 0);
        assert!(match_result.matched_indices.is_empty());
    }

    #[test]
    fn test_fuzzy_match_path_simple_filename_match() {
        let result = FileSearchModel::fuzzy_match_path("src/main.rs", "main");

        assert!(result.is_some());
        let match_result = result.unwrap();
        assert!(match_result.score > 0);
        assert!(!match_result.matched_indices.is_empty());

        // Should match characters in "main" within "src/main.rs"
        let expected_indices = vec![4, 5, 6, 7]; // positions of 'm', 'a', 'i', 'n'
        assert_eq!(match_result.matched_indices, expected_indices);
    }

    #[test]
    fn test_fuzzy_match_path_exact_filename_match() {
        let result = FileSearchModel::fuzzy_match_path("src/main.rs", "main");

        assert!(result.is_some());
        let match_result = result.unwrap();
        // Exact filename matches should get a large score boost
        assert!(match_result.score >= 5000);
    }

    #[test]
    fn test_fuzzy_match_path_case_insensitive() {
        let result = FileSearchModel::fuzzy_match_path("src/Main.rs", "main");

        assert!(result.is_some());
        let match_result = result.unwrap();
        assert!(match_result.score > 0);
        assert!(!match_result.matched_indices.is_empty());
    }

    #[test]
    fn test_fuzzy_match_path_partial_match() {
        let result = FileSearchModel::fuzzy_match_path("src/components/button.tsx", "btn");

        assert!(result.is_some());
        let match_result = result.unwrap();
        assert!(match_result.score > 0);
        // Should match 'b', 't' from "button"
        assert!(match_result.matched_indices.len() >= 2);
    }

    #[test]
    fn test_fuzzy_match_path_no_match() {
        let result = FileSearchModel::fuzzy_match_path("src/main.rs", "xyz");

        assert!(result.is_none());
    }

    #[test]
    fn test_fuzzy_match_path_multi_term_search() {
        let result = FileSearchModel::fuzzy_match_path("src/components/button.tsx", "comp btn");

        assert!(result.is_some());
        let match_result = result.unwrap();
        assert!(match_result.score > 0);
        // Should match characters from both "comp" and "btn"
        assert!(match_result.matched_indices.len() >= 6);
    }

    #[test]
    fn test_fuzzy_match_path_multi_term_partial_match() {
        let result = FileSearchModel::fuzzy_match_path("src/components/button.tsx", "comp xyz");

        assert!(result.is_none());
    }

    #[test]
    fn test_fuzzy_match_path_wildcard_patterns() {
        // Test * wildcard for file extension
        let result = FileSearchModel::fuzzy_match_path("src/main.rs", "*.rs");
        assert!(result.is_some());

        // Test * wildcard for filename
        let result = FileSearchModel::fuzzy_match_path("src/main.rs", "main*");
        assert!(result.is_some());

        // Test ? wildcard for single character
        let result = FileSearchModel::fuzzy_match_path("src/main.rs", "main.?s");
        assert!(result.is_some());

        // Test patterns that should not match
        let result = FileSearchModel::fuzzy_match_path("src/main.rs", "*.tsx");
        assert!(result.is_none());
    }

    #[test]
    fn test_should_skip_overly_broad_query() {
        // Single wildcards should be skipped
        assert!(FileSearchModel::should_skip_overly_broad_query("*"));
        assert!(FileSearchModel::should_skip_overly_broad_query("?"));
        assert!(FileSearchModel::should_skip_overly_broad_query("  *  "));

        // Consecutive wildcards should be skipped
        assert!(FileSearchModel::should_skip_overly_broad_query("**"));
        assert!(FileSearchModel::should_skip_overly_broad_query("*?"));
        assert!(FileSearchModel::should_skip_overly_broad_query("?*"));
        assert!(FileSearchModel::should_skip_overly_broad_query("??"));

        // Valid queries should not be skipped
        assert!(!FileSearchModel::should_skip_overly_broad_query("main"));
        assert!(!FileSearchModel::should_skip_overly_broad_query("*.rs"));
        assert!(!FileSearchModel::should_skip_overly_broad_query("src/main"));
        assert!(!FileSearchModel::should_skip_overly_broad_query("a*b"));
    }

    #[cfg(feature = "local_fs")]
    #[test]
    fn test_contents_args_keeps_folders_by_default() {
        // The default (directory-inclusive) path is used by callers like the
        // AI context menu and must keep folders in the traversal.
        let args = FileSearchModel::contents_args("src", true, |_| Some("src/main.rs".into()));
        assert!(args.include_folders);

        let empty = FileSearchModel::contents_args("", true, |_| Some("src/main.rs".into()));
        assert!(empty.include_folders);
    }

    #[cfg(feature = "local_fs")]
    #[test]
    fn test_contents_args_excludes_folders_for_file_only_search() {
        let args = FileSearchModel::contents_args("src", false, |_| Some("src/main.rs".into()));
        assert!(!args.include_folders);

        let empty = FileSearchModel::contents_args("", false, |_| Some("src/main.rs".into()));
        assert!(!empty.include_folders);
    }

    #[test]
    fn test_filename_prioritization() {
        // Filename matches should score higher than path matches
        let filename_match =
            FileSearchModel::fuzzy_match_path("long/path/to/test.rs", "test").unwrap();
        let path_match =
            FileSearchModel::fuzzy_match_path("test/path/to/other.rs", "test").unwrap();

        // Filename match should have higher score due to 2x multiplier
        assert!(filename_match.score > path_match.score);
    }

    #[test]
    fn test_path_with_extension_exact_match() {
        let result = FileSearchModel::fuzzy_match_path("src/test.rs", "test");

        assert!(result.is_some());
        let match_result = result.unwrap();
        // Should get exact match bonus even with extension
        assert!(match_result.score >= 5000);
    }
}

#[cfg(test)]
mod file_search_item_tests {
    use super::*;

    #[test]
    fn test_file_search_item_from_result() {
        let result = FileSearchResult {
            path: "src/main.rs".to_string(),
            project_directory: "/Users/test_user/project".to_string(),
            is_directory: false,
        };

        let match_result = FuzzyMatchResult {
            score: 100,
            matched_indices: vec![0, 1, 2],
        };

        let item = FileSearchItem::from_result(result, match_result);

        assert_eq!(item.path, "src/main.rs");
        assert!(!item.is_directory);
        assert_eq!(item.match_result.score, 100);
        assert_eq!(item.match_result.matched_indices, vec![0, 1, 2]);
    }

    #[test]
    fn test_file_search_item_from_result_no_match() {
        let result = FileSearchResult {
            path: "src/test.rs".to_string(),
            project_directory: "/Users/test_user/project".to_string(),
            is_directory: false,
        };

        let item = FileSearchItem::from_result_no_match(result);

        assert_eq!(item.path, "src/test.rs");
        assert!(!item.is_directory);
        assert_eq!(item.match_result.score, 0);
        assert!(item.match_result.matched_indices.is_empty());
    }

    #[test]
    fn test_file_search_item_directory() {
        let result = FileSearchResult {
            path: "src/components".to_string(),
            project_directory: "/Users/test_user/project".to_string(),
            is_directory: true,
        };

        let item = FileSearchItem::from_result_no_match(result);

        assert_eq!(item.path, "src/components");
        assert!(item.is_directory);
    }

    #[test]
    fn test_file_search_item_clone() {
        let result = FileSearchResult {
            path: "test.txt".to_string(),
            project_directory: "/Users/test_user/project".to_string(),
            is_directory: false,
        };

        let original = FileSearchItem::from_result_no_match(result);
        let cloned = original.clone();

        assert_eq!(original.path, cloned.path);
        assert_eq!(original.is_directory, cloned.is_directory);
        assert_eq!(original.match_result.score, cloned.match_result.score);
        assert_eq!(
            original.match_result.matched_indices,
            cloned.match_result.matched_indices
        );
    }
}

#[cfg(test)]
mod strip_absolute_path_prefix_tests {
    use std::path::{Path, PathBuf};

    use super::*;

    /// Builds an absolute path from the given components, using the platform's
    /// root (`/` on Unix, `C:\` on Windows).  This ensures the constructed
    /// path is treated as absolute by `Path::is_absolute` on both platforms.
    fn abs_path(components: &[&str]) -> String {
        let mut path = PathBuf::new();
        #[cfg(windows)]
        path.push(r"C:\");
        #[cfg(unix)]
        path.push("/");
        for component in components {
            path.push(component);
        }
        path.to_string_lossy().into_owned()
    }

    /// Builds a relative path using the platform's native separator, so tests
    /// can compare against `strip_prefix` output without hardcoding `/` or `\`.
    fn rel_path(components: &[&str]) -> String {
        components
            .iter()
            .collect::<PathBuf>()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn test_relative_path_unchanged() {
        let repo_root = abs_path(&["home", "user", "project"]);
        let result = FileSearchModel::strip_absolute_path_prefix(
            "src/main.rs",
            Some(Path::new(&repo_root)),
            None,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_strips_repo_root() {
        let abs = abs_path(&["home", "user", "project", "src", "main.rs"]);
        let repo_root = abs_path(&["home", "user", "project"]);
        let expected = rel_path(&["src", "main.rs"]);
        let result =
            FileSearchModel::strip_absolute_path_prefix(&abs, Some(Path::new(&repo_root)), None);
        assert_eq!(result.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn test_strips_working_dir_when_no_repo_root() {
        let abs = abs_path(&["home", "user", "project", "src", "main.rs"]);
        let working_dir = abs_path(&["home", "user", "project"]);
        let expected = rel_path(&["src", "main.rs"]);
        let result =
            FileSearchModel::strip_absolute_path_prefix(&abs, None, Some(Path::new(&working_dir)));
        assert_eq!(result.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn test_prefers_repo_root_over_working_dir() {
        let abs = abs_path(&["repo", "src", "main.rs"]);
        let repo_root = abs_path(&["repo"]);
        let working_dir = abs_path(&["other"]);
        let expected = rel_path(&["src", "main.rs"]);
        let result = FileSearchModel::strip_absolute_path_prefix(
            &abs,
            Some(Path::new(&repo_root)),
            Some(Path::new(&working_dir)),
        );
        assert_eq!(result.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn test_falls_back_to_working_dir_when_repo_root_does_not_match() {
        let abs = abs_path(&["home", "user", "project", "src", "main.rs"]);
        let repo_root = abs_path(&["other", "repo"]);
        let working_dir = abs_path(&["home", "user", "project"]);
        let expected = rel_path(&["src", "main.rs"]);
        let result = FileSearchModel::strip_absolute_path_prefix(
            &abs,
            Some(Path::new(&repo_root)),
            Some(Path::new(&working_dir)),
        );
        assert_eq!(result.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn test_returns_none_when_no_prefix_matches() {
        let abs = abs_path(&["home", "user", "project", "src", "main.rs"]);
        let repo_root = abs_path(&["other", "repo"]);
        let working_dir = abs_path(&["another", "dir"]);
        let result = FileSearchModel::strip_absolute_path_prefix(
            &abs,
            Some(Path::new(&repo_root)),
            Some(Path::new(&working_dir)),
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_returns_none_when_path_equals_prefix() {
        // Exact match with no remaining path should return None.
        let abs = abs_path(&["home", "user", "project"]);
        let result = FileSearchModel::strip_absolute_path_prefix(&abs, Some(Path::new(&abs)), None);
        assert!(result.is_none());
    }

    #[test]
    fn test_no_roots_provided() {
        let abs = abs_path(&["home", "user", "project", "src", "main.rs"]);
        let result = FileSearchModel::strip_absolute_path_prefix(&abs, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_working_dir_inside_repo_root_strips_repo_root() {
        // When the working directory is a subdirectory of the repo root, the
        // returned path is relative to the repo root (not the working dir),
        // since repo_root is tried first.  This keeps query normalization
        // aligned with how the file index stores paths.
        let abs = abs_path(&["home", "user", "project", "src", "main.rs"]);
        let repo_root = abs_path(&["home", "user", "project"]);
        let working_dir = abs_path(&["home", "user", "project", "src"]);
        let expected = rel_path(&["src", "main.rs"]);
        let result = FileSearchModel::strip_absolute_path_prefix(
            &abs,
            Some(Path::new(&repo_root)),
            Some(Path::new(&working_dir)),
        );
        assert_eq!(result.as_deref(), Some(expected.as_str()));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_typical_search_workflow() {
        // Simulate a typical search workflow
        let paths = vec![
            "src/main.rs",
            "src/lib.rs",
            "src/components/button.tsx",
            "src/components/input.tsx",
            "tests/integration_tests.rs",
            "README.md",
        ];

        let query = "main";

        let mut results = Vec::new();
        for path in paths {
            if let Some(match_result) = FileSearchModel::fuzzy_match_path(path, query) {
                let search_result = FileSearchResult {
                    path: path.to_string(),
                    project_directory: "/Users/test_user/project".to_string(),
                    is_directory: false,
                };
                let search_item = FileSearchItem::from_result(search_result, match_result);
                results.push(search_item);
            }
        }

        // Should find "src/main.rs"
        assert!(!results.is_empty());

        // Sort by score (highest first)
        results.sort_by(|a, b| b.match_result.score.cmp(&a.match_result.score));

        // "src/main.rs" should be the top result (exact filename match)
        assert_eq!(results[0].path, "src/main.rs");
        assert!(results[0].match_result.score >= 5000); // Should have exact match bonus
    }

    #[test]
    fn test_multi_term_search_workflow() {
        let paths = vec![
            "src/components/button.tsx",
            "src/components/input.tsx",
            "src/utils/button_helper.rs",
            "tests/button_tests.rs",
        ];

        let query = "comp button";

        let mut results = Vec::new();
        for path in paths {
            if let Some(match_result) = FileSearchModel::fuzzy_match_path(path, query) {
                let search_result = FileSearchResult {
                    path: path.to_string(),
                    project_directory: "/Users/test_user/project".to_string(),
                    is_directory: false,
                };
                results.push(FileSearchItem::from_result(search_result, match_result));
            }
        }

        // Should find "src/components/button.tsx" as it matches both terms
        assert!(!results.is_empty());

        let button_component = results
            .iter()
            .find(|item| item.path == "src/components/button.tsx");
        assert!(button_component.is_some());
    }
}
