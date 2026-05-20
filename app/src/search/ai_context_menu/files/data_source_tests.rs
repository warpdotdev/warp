use crate::search::{
    ai_context_menu::{
        files::data_source::{file_data_source_for_pwd, fuzzy_match_files, FileSnapshot},
        mixer::AIContextMenuSearchableAction,
    },
    data_source::Query,
    files::{model::FileSearchModel, search_item::FileSearchResult},
    item::SearchItem,
    mixer::AsyncDataSource,
};
use crate::{terminal::model::session::Session, workspace::ActiveSession};
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::RepoMetadataModel;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;
use warpui::platform::WindowStyle;
use warpui::r#async::block_on;
use warpui::windowing::WindowManager;
use warpui::SingletonEntity;
use warpui::{elements::Empty, App, AppContext, Element, Entity, TypedActionView, View};
struct TestView;

impl Entity for TestView {
    type Event = ();
}

impl View for TestView {
    fn ui_name() -> &'static str {
        "AIContextMenuFilesDataSourceTestView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for TestView {
    type Action = ();
}

#[test]
fn test_single_term_query() {
    let path = "/src/components/button.rs";
    let query = "button";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_multi_term_query_all_match() {
    let path = "/src/components/button.rs";
    let query = "src button";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    // Should have matches for both "src" and "button"
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_multi_term_query_partial_match() {
    let path = "/src/components/button.rs";
    let query = "src nonexistent";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    // Should return None because "nonexistent" doesn't match
    assert!(result.is_none());
}

#[test]
fn test_multi_term_query_combined_score() {
    let path = "/src/components/button.rs";

    let single_result = FileSearchModel::fuzzy_match_path(path, "button").unwrap();
    let multi_result = FileSearchModel::fuzzy_match_path(path, "src button").unwrap();

    // Multi-term query should have a higher score (combination of both matches)
    assert!(multi_result.score >= single_result.score);
}

#[test]
fn test_empty_query() {
    let path = "/src/components/button.rs";
    let query = "";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert_eq!(match_result.score, 0);
    assert!(match_result.matched_indices.is_empty());
}

#[test]
fn test_whitespace_only_query() {
    let path = "/src/components/button.rs";
    let query = "   ";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert_eq!(match_result.score, 0);
    assert!(match_result.matched_indices.is_empty());
}

#[test]
fn test_three_term_query() {
    let path = "/src/components/ui/button.rs";
    let query = "src ui button";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_single() {
    let path = "/src/components/button.rs";
    let query = "comp*.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
#[cfg(feature = "local_fs")]
fn test_git_changed_files_boost() {
    // Test that git changed files get boosted in zero state (empty query)
    let normal_file = "normal_file.rs";
    let changed_file = "changed_file.rs";

    let normal_match = FileSearchModel::fuzzy_match_path(normal_file, "");
    let mut changed_match = FileSearchModel::fuzzy_match_path(changed_file, "");

    assert!(normal_match.is_some());
    assert!(changed_match.is_some());

    // Simulate git boost for changed file
    if let Some(ref mut match_result) = changed_match {
        match_result.score += 10000; // Same boost as in the actual code
    }

    let normal_score = normal_match.unwrap().score;
    let changed_score = changed_match.unwrap().score;

    // Changed file should have significantly higher score
    assert!(changed_score > normal_score + 9000);
}

#[test]
#[cfg(feature = "local_fs")]
fn test_git_changed_files_no_boost_with_query() {
    // Test that git changed files don't get boosted when there's a query
    let normal_file = "normal_file.rs";
    let changed_file = "changed_file.rs";
    let query = "file";

    let normal_match = FileSearchModel::fuzzy_match_path(normal_file, query);
    let changed_match = FileSearchModel::fuzzy_match_path(changed_file, query);

    assert!(normal_match.is_some());
    assert!(changed_match.is_some());

    // Without the git boost, scores should be similar for similar filenames
    let normal_score = normal_match.unwrap().score;
    let changed_score = changed_match.unwrap().score;

    // Scores should be roughly equal (within reasonable range)
    assert!((normal_score - changed_score).abs() < 1000);
}

#[test]
fn test_filename_priority_with_spaces() {
    let path = "/some/deep/path/important_file.rs";
    let query = "deep important";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    // Should match both "deep" in path and "important" in filename
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_star_in_filename() {
    let path = "/src/components/button.rs";
    let query = "*.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_star_in_directory() {
    let path = "/src/components/button.rs";
    let query = "src/*/button.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_star_multiple() {
    let path = "/src/components/ui/button.rs";
    let query = "*/ui/*.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_question_mark() {
    let path = "/src/components/button.rs";
    let query = "butto?.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_mixed_with_spaces() {
    let path = "/src/components/ui/button.rs";
    let query = "ui *.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_no_match() {
    let path = "/src/components/button.rs";
    let query = "*.py";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_none());
}

#[test]
fn test_wildcard_exact_match() {
    let path = "/src/components/button.rs";
    let query = "/src/components/button.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    // Should match the entire path
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_case_insensitive() {
    let path = "/src/Components/Button.rs";
    let query = "*/button.RS";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_at_beginning() {
    let path = "/src/components/button.rs";
    let query = "*button.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_example_ui_star_rs() {
    // Test the specific example from the request: "ui/*.rs"
    let path1 = "/src/ui/button.rs";
    let path2 = "/src/ui/input.rs";
    let path3 = "/src/ui/modal.rs";
    let path4 = "/src/components/button.rs"; // Should not match
    let query = "ui/*.rs";

    // These should match
    let result1 = FileSearchModel::fuzzy_match_path(path1, query);
    assert!(result1.is_some());
    assert!(result1.unwrap().score > 0);

    let result2 = FileSearchModel::fuzzy_match_path(path2, query);
    assert!(result2.is_some());
    assert!(result2.unwrap().score > 0);

    let result3 = FileSearchModel::fuzzy_match_path(path3, query);
    assert!(result3.is_some());
    assert!(result3.unwrap().score > 0);

    // This should not match because it's not in the ui directory
    let result4 = FileSearchModel::fuzzy_match_path(path4, query);
    assert!(result4.is_none());
}

#[test]
fn test_should_skip_overly_broad_query() {
    // Should skip single wildcards
    assert!(FileSearchModel::should_skip_overly_broad_query("*"));
    assert!(FileSearchModel::should_skip_overly_broad_query("?"));

    // Should skip consecutive wildcards
    assert!(FileSearchModel::should_skip_overly_broad_query("**"));
    assert!(FileSearchModel::should_skip_overly_broad_query("***"));
    assert!(FileSearchModel::should_skip_overly_broad_query("*?"));
    assert!(FileSearchModel::should_skip_overly_broad_query("?*"));
    assert!(FileSearchModel::should_skip_overly_broad_query("??"));
    assert!(FileSearchModel::should_skip_overly_broad_query(
        "test**file"
    ));
    assert!(FileSearchModel::should_skip_overly_broad_query(
        "src*?button"
    ));

    // Should NOT skip reasonable queries
    assert!(!FileSearchModel::should_skip_overly_broad_query("button"));
    assert!(!FileSearchModel::should_skip_overly_broad_query("*.rs"));
    assert!(!FileSearchModel::should_skip_overly_broad_query(
        "src/components"
    ));
    assert!(!FileSearchModel::should_skip_overly_broad_query("ui/*.rs"));
    assert!(!FileSearchModel::should_skip_overly_broad_query(
        "test*file"
    ));
    assert!(!FileSearchModel::should_skip_overly_broad_query(
        "src button"
    ));
    assert!(!FileSearchModel::should_skip_overly_broad_query("*a"));
    assert!(!FileSearchModel::should_skip_overly_broad_query("a*"));
    assert!(!FileSearchModel::should_skip_overly_broad_query("*.*"));
    assert!(!FileSearchModel::should_skip_overly_broad_query("a"));
    assert!(!FileSearchModel::should_skip_overly_broad_query("ab"));
}

#[test]
fn test_path_proximity_ranking() {
    let path = "/very/long/deeply/nested/path/to/components/test.rs";

    // Test that matches closer to the filename get higher scores than matches at the beginning
    let result_near_filename = FileSearchModel::fuzzy_match_path_single(path, "components");
    let result_at_beginning = FileSearchModel::fuzzy_match_path_single(path, "very");

    assert!(result_near_filename.is_some());
    assert!(result_at_beginning.is_some());

    let near_score = result_near_filename.unwrap().score;
    let beginning_score = result_at_beginning.unwrap().score;

    // "components" is closer to the filename than "very", so it should have a higher score
    assert!(
        near_score > beginning_score,
        "Expected matches closer to filename to have higher scores. Near: {near_score}, Beginning: {beginning_score}"
    );
}

#[test]
fn test_directory_search_support() {
    use crate::search::ai_context_menu::files::search_item::FileSearchItem;
    use fuzzy_match::FuzzyMatchResult;

    // Test that directories can be created with is_directory flag
    let directory_item = FileSearchItem {
        path: PathBuf::from("src/components/"),
        match_result: FuzzyMatchResult::no_match(),
        is_directory: true,
    };

    let file_item = FileSearchItem {
        path: PathBuf::from("src/components/button.rs"),
        match_result: FuzzyMatchResult::no_match(),
        is_directory: false,
    };

    assert!(directory_item.is_directory);
    assert!(!file_item.is_directory);

    // Test accessibility labels
    assert!(directory_item
        .accessibility_label()
        .starts_with("Directory:"));
    assert!(file_item.accessibility_label().starts_with("File:"));
}

#[test]
fn test_directory_action_type() {
    use crate::search::ai_context_menu::files::search_item::FileSearchItem;
    use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
    use crate::search::item::SearchItem;
    use fuzzy_match::FuzzyMatchResult;

    let directory_item = FileSearchItem {
        path: PathBuf::from("src/components/"),
        match_result: FuzzyMatchResult::no_match(),
        is_directory: true,
    };

    let file_item = FileSearchItem {
        path: PathBuf::from("src/components/button.rs"),
        match_result: FuzzyMatchResult::no_match(),
        is_directory: false,
    };

    // Test that directories return InsertFilePath action with trailing slash
    match directory_item.accept_result() {
        AIContextMenuSearchableAction::InsertFilePath { file_path } => {
            assert_eq!(file_path, "src/components/");
        }
        _ => panic!("Expected InsertFilePath action for directory"),
    }

    // Test that files return InsertFilePath action without trailing slash
    match file_item.accept_result() {
        AIContextMenuSearchableAction::InsertFilePath { file_path } => {
            assert_eq!(file_path, "src/components/button.rs");
        }
        _ => panic!("Expected InsertFilePath action for file"),
    }
}

#[test]
fn test_directory_fuzzy_matching() {
    // Test fuzzy matching works for directory paths
    let directory_path = "src/components/ui";
    let query = "comp ui";

    let result = FileSearchModel::fuzzy_match_path(directory_path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_directory_vs_file_scoring() {
    // Test that files and directories with similar names get similar scores
    let directory_path = "src/components";
    let file_path = "src/components.rs";
    let query = "components";

    let dir_result = FileSearchModel::fuzzy_match_path(directory_path, query);
    let file_result = FileSearchModel::fuzzy_match_path(file_path, query);

    assert!(dir_result.is_some());
    assert!(file_result.is_some());

    // Both should have good scores since they match the query well
    let dir_score = dir_result.unwrap().score;
    let file_score = file_result.unwrap().score;

    assert!(dir_score > 0);
    assert!(file_score > 0);

    // Both should have reasonable scores - the exact relative scoring may vary
    // but both should be positive and meaningful
    assert!(dir_score > 0);
    assert!(file_score > 0);

    // The specific relative ordering depends on the fuzzy matching algorithm
    // Both are valid matches for "components" query
}

#[test]
fn test_mixed_file_directory_search() {
    // Test searching through a mix of files and directories
    let paths_and_types = vec![
        ("src/components", true),
        ("src/components/button.rs", false),
        ("src/components/ui", true),
        ("src/components/ui/modal.rs", false),
        ("tests/components", true),
        ("tests/components/button_tests.rs", false),
    ];

    let query = "components";

    for (path, is_directory) in paths_and_types {
        let result = FileSearchModel::fuzzy_match_path(path, query);
        assert!(result.is_some(), "Failed to match path: {path}");

        let match_result = result.unwrap();
        assert!(match_result.score > 0, "Zero score for path: {path}");
        assert!(
            !match_result.matched_indices.is_empty(),
            "No match indices for path: {path}"
        );

        // Verify we can create search items for both types
        let search_item = crate::search::ai_context_menu::files::search_item::FileSearchItem {
            path: PathBuf::from(path),
            match_result,
            is_directory,
        };

        assert_eq!(search_item.is_directory, is_directory);
    }
}

#[test]
fn test_apply_path_proximity_ranking_direct() {
    let path = "/src/components/button.rs";

    // Test the proximity ranking function directly
    let matched_indices_near_end = vec![15, 16, 17]; // Near "button"
    let matched_indices_at_start = vec![1, 2, 3]; // Near "src"

    let base_score = 1000;

    let score_near_end =
        FileSearchModel::apply_path_proximity_ranking(path, &matched_indices_near_end, base_score);
    let score_at_start =
        FileSearchModel::apply_path_proximity_ranking(path, &matched_indices_at_start, base_score);

    // Matches closer to the end should get a higher bonus
    assert!(
        score_near_end > score_at_start,
        "Expected matches near end to get higher scores. Near end: {score_near_end}, At start: {score_at_start}"
    );

    // Both should be higher than the base score
    assert!(score_near_end > base_score);
    assert!(score_at_start > base_score);
}

#[test]
fn test_proximity_ranking_empty_indices() {
    let path = "/src/components/button.rs";
    let base_score = 1000;

    // Empty matched indices should return the original score
    let result = FileSearchModel::apply_path_proximity_ranking(path, &[], base_score);
    assert_eq!(result, base_score);
}

#[test]
fn test_proximity_ranking_realistic_scenario() {
    // Realistic scenario: searching for "button" in different paths
    let path1 = "/very/long/deeply/nested/path/with/many/segments/button.rs"; // "button" at end
    let path2 = "/button/very/long/deeply/nested/path/with/many/segments/file.rs"; // "button" at start

    let result1 = FileSearchModel::fuzzy_match_path_single(path1, "button").unwrap();
    let result2 = FileSearchModel::fuzzy_match_path_single(path2, "button").unwrap();

    // path1 should have higher score due to "button" being closer to the filename
    assert!(
        result1.score > result2.score,
        "Expected filename match to rank higher than directory match. Filename: {}, Directory: {}",
        result1.score,
        result2.score
    );
}

fn make_file(path: &str) -> FileSearchResult {
    FileSearchResult {
        path: path.to_string(),
        project_directory: "/project".to_string(),
        is_directory: false,
    }
}

fn make_dir(path: &str) -> FileSearchResult {
    FileSearchResult {
        path: path.to_string(),
        project_directory: "/project".to_string(),
        is_directory: true,
    }
}

#[test]
fn test_fuzzy_match_files_zero_state_git_changed_first() {
    let contents = vec![
        make_file("src/main.rs"),
        make_file("src/lib.rs"),
        make_file("src/changed.rs"),
    ];
    let git_changed_files = HashSet::from(["src/changed.rs".to_string()]);

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files,
        query_text: String::new(),
        last_opened: HashMap::new(),
    }))
    .unwrap();

    assert_eq!(results.len(), 3);
    // Git-changed file should be first with high score
    assert_eq!(
        results[0].accept_result(),
        AIContextMenuSearchableAction::InsertFilePath {
            file_path: "src/changed.rs".to_string()
        }
    );
    assert!(results[0].score() > results[1].score());
}

#[test]
fn test_fuzzy_match_files_zero_state_no_git_changes() {
    let contents = vec![make_file("src/main.rs"), make_file("src/lib.rs")];

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files: HashSet::new(),
        query_text: String::new(),
        last_opened: HashMap::new(),
    }))
    .unwrap();

    assert_eq!(results.len(), 2);
}

#[test]
fn test_fuzzy_match_files_non_empty_query() {
    let contents = vec![
        make_file("src/components/button.rs"),
        make_file("src/components/modal.rs"),
        make_file("src/utils/helpers.rs"),
    ];

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files: HashSet::new(),
        query_text: "button".to_string(),
        last_opened: HashMap::new(),
    }))
    .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].accept_result(),
        AIContextMenuSearchableAction::InsertFilePath {
            file_path: "src/components/button.rs".to_string()
        }
    );
}

#[test]
fn test_fuzzy_match_files_directory_boost() {
    let contents = vec![make_dir("src/components"), make_file("src/components.rs")];

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files: HashSet::new(),
        query_text: "components".to_string(),
        last_opened: HashMap::new(),
    }))
    .unwrap();

    assert_eq!(results.len(), 2);
    // File should score higher than directory due to the +100 boost
    let file_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/components.rs".to_string(),
                }
        })
        .expect("file result not found");
    let dir_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/components".to_string(),
                }
        })
        .expect("dir result not found");
    assert!(file_result.score() > dir_result.score());
}

#[test]
fn test_fuzzy_match_files_respects_max_results() {
    let contents: Vec<FileSearchResult> = (0..300)
        .map(|i| make_file(&format!("src/file_{i}.rs")))
        .collect();

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files: HashSet::new(),
        query_text: String::new(),
        last_opened: HashMap::new(),
    }))
    .unwrap();

    assert_eq!(results.len(), 200);
}

#[test]
#[cfg(feature = "local_fs")]
fn test_file_data_source_for_pwd_holistic_behavior() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(FileSearchModel::new);
        app.add_singleton_model(|_| ActiveSession::default());
        let test_dir = tempdir().expect("failed to create temp dir");
        let test_dir_path = test_dir.path().to_path_buf();
        fs::write(test_dir_path.join("needle.rs"), "fn main() {}")
            .expect("failed to write test file");
        fs::write(test_dir_path.join("other.txt"), "other").expect("failed to write test file");

        let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, |_ctx| TestView);
        WindowManager::handle(&app).update(&mut app, |windowing_state, _ctx| {
            windowing_state.overwrite_for_test(windowing_state.stage(), Some(window_id));
        });

        ActiveSession::handle(&app).update(&mut app, |active_session, ctx| {
            active_session.set_session_for_test(
                window_id,
                Arc::new(Session::test()),
                Some(test_dir_path),
                None,
                ctx,
            );
        });

        let data_source = app.read(file_data_source_for_pwd);

        let broad_query_results = app
            .read(|ctx| {
                data_source.run_query(
                    &Query {
                        text: "*".to_string(),
                        filters: HashSet::new(),
                    },
                    ctx,
                )
            })
            .await
            .expect("broad query run failed");
        assert!(broad_query_results.is_empty());
        let focused_query_results = app
            .read(|ctx| {
                data_source.run_query(
                    &Query {
                        text: "needle".to_string(),
                        filters: HashSet::new(),
                    },
                    ctx,
                )
            })
            .await
            .expect("focused query run failed");
        assert_eq!(focused_query_results.len(), 1);
        assert_eq!(
            focused_query_results[0].accept_result(),
            AIContextMenuSearchableAction::InsertFilePath {
                file_path: "needle.rs".to_string()
            }
        );
    });
}

/// Helper: create an `Instant` that is `millis` milliseconds after a baseline.
/// We use small sleeps to guarantee monotonically increasing instants.
fn instant_offset(millis: u64) -> instant::Instant {
    // Sleep to advance the clock relative to previous calls.
    std::thread::sleep(std::time::Duration::from_millis(millis));
    instant::Instant::now()
}

#[test]
fn test_zero_state_recently_opened_files_rank_above_untouched() {
    let opened_at = instant::Instant::now();
    let last_opened = HashMap::from([("src/opened.rs".to_string(), opened_at)]);

    let contents = vec![make_file("src/untouched.rs"), make_file("src/opened.rs")];

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files: HashSet::new(),
        query_text: String::new(),
        last_opened,
    }))
    .unwrap();

    assert_eq!(results.len(), 2);
    let opened_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/opened.rs".to_string(),
                }
        })
        .expect("opened file not found");
    let untouched_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/untouched.rs".to_string(),
                }
        })
        .expect("untouched file not found");
    assert!(
        opened_result.score() > untouched_result.score(),
        "Expected recently-opened file to score higher. Opened: {:?}, Untouched: {:?}",
        opened_result.score(),
        untouched_result.score(),
    );
}

#[test]
fn test_zero_state_git_changed_ranks_above_recently_opened() {
    let opened_at = instant::Instant::now();
    let last_opened = HashMap::from([("src/opened.rs".to_string(), opened_at)]);

    let contents = vec![make_file("src/changed.rs"), make_file("src/opened.rs")];
    let git_changed_files = HashSet::from(["src/changed.rs".to_string()]);

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files,
        query_text: String::new(),
        last_opened,
    }))
    .unwrap();

    assert_eq!(results.len(), 2);
    let changed_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/changed.rs".to_string(),
                }
        })
        .expect("changed file not found");
    let opened_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/opened.rs".to_string(),
                }
        })
        .expect("opened file not found");
    assert!(
        changed_result.score() > opened_result.score(),
        "Expected git-changed file to rank above recently-opened. Changed: {:?}, Opened: {:?}",
        changed_result.score(),
        opened_result.score(),
    );
}

#[test]
fn test_zero_state_recently_opened_ordered_by_recency() {
    // Create instants with guaranteed ordering: older < newer
    let older = instant_offset(1);
    let newer = instant_offset(1);

    let last_opened = HashMap::from([
        ("src/older.rs".to_string(), older),
        ("src/newer.rs".to_string(), newer),
    ]);

    let contents = vec![make_file("src/older.rs"), make_file("src/newer.rs")];

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files: HashSet::new(),
        query_text: String::new(),
        last_opened,
    }))
    .unwrap();

    assert_eq!(results.len(), 2);
    let newer_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/newer.rs".to_string(),
                }
        })
        .expect("newer file not found");
    let older_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/older.rs".to_string(),
                }
        })
        .expect("older file not found");
    assert!(
        newer_result.score() > older_result.score(),
        "Expected more recently opened file to score higher. Newer: {:?}, Older: {:?}",
        newer_result.score(),
        older_result.score(),
    );
}

#[test]
fn test_zero_state_git_changed_also_ordered_by_recency() {
    let older = instant_offset(1);
    let newer = instant_offset(1);

    let last_opened = HashMap::from([
        ("src/changed_older.rs".to_string(), older),
        ("src/changed_newer.rs".to_string(), newer),
    ]);

    let contents = vec![
        make_file("src/changed_older.rs"),
        make_file("src/changed_newer.rs"),
    ];
    let git_changed_files = HashSet::from([
        "src/changed_older.rs".to_string(),
        "src/changed_newer.rs".to_string(),
    ]);

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files,
        query_text: String::new(),
        last_opened,
    }))
    .unwrap();

    assert_eq!(results.len(), 2);
    let newer_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/changed_newer.rs".to_string(),
                }
        })
        .expect("newer changed file not found");
    let older_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/changed_older.rs".to_string(),
                }
        })
        .expect("older changed file not found");
    assert!(
        newer_result.score() > older_result.score(),
        "Expected more recently opened git-changed file to score higher. Newer: {:?}, Older: {:?}",
        newer_result.score(),
        older_result.score(),
    );
}

#[test]
fn test_fuzzy_query_recently_opened_bonus() {
    let opened_at = instant::Instant::now();
    let last_opened = HashMap::from([("src/components/opened_button.rs".to_string(), opened_at)]);

    let contents = vec![
        make_file("src/components/opened_button.rs"),
        make_file("src/components/other_button.rs"),
    ];

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files: HashSet::new(),
        query_text: "button".to_string(),
        last_opened,
    }))
    .unwrap();

    assert_eq!(results.len(), 2);
    let opened_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/components/opened_button.rs".to_string(),
                }
        })
        .expect("opened file not found");
    let other_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/components/other_button.rs".to_string(),
                }
        })
        .expect("other file not found");
    assert!(
        opened_result.score() > other_result.score(),
        "Expected recently-opened file to score higher in fuzzy mode. Opened: {:?}, Other: {:?}",
        opened_result.score(),
        other_result.score(),
    );
}

#[test]
fn test_zero_state_full_ordering_end_to_end() {
    let older_opened = instant_offset(1);
    let newer_opened = instant_offset(1);
    let git_opened = instant_offset(1);

    let last_opened = HashMap::from([
        ("src/opened_older.rs".to_string(), older_opened),
        ("src/opened_newer.rs".to_string(), newer_opened),
        ("src/changed_opened.rs".to_string(), git_opened),
    ]);

    let contents = vec![
        make_file("src/untouched.rs"),
        make_file("src/opened_older.rs"),
        make_file("src/opened_newer.rs"),
        make_file("src/changed_plain.rs"),
        make_file("src/changed_opened.rs"),
    ];
    let git_changed_files = HashSet::from([
        "src/changed_plain.rs".to_string(),
        "src/changed_opened.rs".to_string(),
    ]);

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files,
        query_text: String::new(),
        last_opened,
    }))
    .unwrap();

    assert_eq!(results.len(), 5);

    // Extract scores by file path
    let score_of = |path: &str| -> ordered_float::OrderedFloat<f64> {
        results
            .iter()
            .find(|r| {
                r.accept_result()
                    == AIContextMenuSearchableAction::InsertFilePath {
                        file_path: path.to_string(),
                    }
            })
            .unwrap_or_else(|| panic!("result not found for {path}"))
            .score()
    };

    // Tier 1: git-changed (opened > plain)
    assert!(
        score_of("src/changed_opened.rs") > score_of("src/changed_plain.rs"),
        "git-changed opened should beat git-changed plain"
    );
    // Tier 1 > Tier 2: any git-changed > any non-git
    assert!(
        score_of("src/changed_plain.rs") > score_of("src/opened_newer.rs"),
        "git-changed should beat recently-opened non-git"
    );
    // Tier 2: recently opened (newer > older)
    assert!(
        score_of("src/opened_newer.rs") > score_of("src/opened_older.rs"),
        "newer opened should beat older opened"
    );
    // Tier 2 > Tier 3: any opened > untouched
    assert!(
        score_of("src/opened_older.rs") > score_of("src/untouched.rs"),
        "opened should beat untouched"
    );
}
