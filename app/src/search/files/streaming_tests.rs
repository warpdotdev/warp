use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use itertools::Itertools;
use streaming_file_search::StreamingFileSearchEngine;
use warpui::r#async::block_on;

use super::*;

fn write_file(root: &Path, relative: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, b"contents").unwrap();
}

/// Builds a session directly over `root`, bypassing `for_active_local_repo`
/// (which needs a full `AppContext` and the feature flag).
fn session_for_root(root: &Path, git_changed_files: HashSet<String>) -> StreamingFileSearchSession {
    StreamingFileSearchSession {
        engine: Mutex::new(StreamingFileSearchEngine::new(root.to_path_buf())),
        project_directory: root.to_string_lossy().to_string(),
        git_changed_files,
    }
}

#[test]
fn collect_candidates_zero_state_returns_all() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "src/model.rs");
    write_file(dir.path(), "src/view.rs");

    let session = session_for_root(dir.path(), HashSet::new());
    let results = block_on(session.collect_candidates("", usize::MAX));

    // Two files plus the src/ directory.
    assert_eq!(results.len(), 3);
    for result in &results {
        assert_eq!(result.project_directory, dir.path().to_string_lossy());
    }
}

#[test]
fn collect_candidates_fuzzy_query_filters() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "src/model.rs");
    write_file(dir.path(), "src/view.rs");

    let session = session_for_root(dir.path(), HashSet::new());
    let results = block_on(session.collect_candidates("model", usize::MAX));

    assert_eq!(results.len(), 1);
    assert!(results[0].path.ends_with("model.rs"));
    assert!(!results[0].is_directory);
}

#[test]
fn collect_candidates_wildcard_query_returns_all_for_downstream_matching() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "src/model.rs");
    write_file(dir.path(), "notes.md");

    let session = session_for_root(dir.path(), HashSet::new());
    // Wildcards are not fed to the engine; all candidates come back so the
    // caller's wildcard matcher can filter them.
    let results = block_on(session.collect_candidates("*.rs", usize::MAX));

    assert_eq!(results.len(), 3);
}

#[test]
fn collect_candidates_respects_max_results() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..20 {
        write_file(dir.path(), &format!("file_{i}.rs"));
    }

    let session = session_for_root(dir.path(), HashSet::new());
    let results = block_on(session.collect_candidates("file", 5));
    assert_eq!(results.len(), 5);
}

/// Compares streaming-engine-backed ranking (nucleo recall + `fuzzy_match_path`
/// re-rank) against the ground-truth full scan (`fuzzy_match_path` over every
/// candidate) on a real repository, and prints ordering differences.
///
/// Run manually with:
/// `cargo test -p warp --lib streaming_ranking_parity_report -- --ignored --nocapture`
/// Optionally set `WARP_SEARCH_PARITY_ROOT` to point at a repo (defaults to
/// this repository).
#[test]
#[ignore = "manual parity report against a real repository"]
fn streaming_ranking_parity_report() {
    const TOP_N: usize = 20;
    const OVERFETCH: usize = 1000;

    let root = std::env::var("WARP_SEARCH_PARITY_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_path_buf()
        });
    let session = session_for_root(&root, HashSet::new());
    wait_for_scan(&session);

    // Ground truth: every candidate, matched with fuzzy_match_path.
    let all_candidates = block_on(session.collect_candidates("", usize::MAX));
    println!(
        "parity root: {} ({} candidates)",
        root.display(),
        all_candidates.len()
    );

    for query in [
        "model",
        "data source",
        "cargo",
        "readme",
        "searchmod",
        "tests",
        "lib.rs",
        "wgsl",
    ] {
        let baseline: Vec<String> = all_candidates
            .iter()
            .filter(|item| !item.is_directory)
            .filter_map(|item| {
                FileSearchModel::fuzzy_match_path(&item.path, query)
                    .map(|m| (item.path.clone(), m.score))
            })
            .k_largest_relaxed_by_key(TOP_N, |(_, score)| *score)
            .map(|(path, _)| path)
            .collect();

        let streaming: Vec<String> = block_on(session.collect_candidates(query, OVERFETCH))
            .into_iter()
            .filter(|item| !item.is_directory)
            .filter_map(|item| {
                FileSearchModel::fuzzy_match_path(&item.path, query).map(|m| (item.path, m.score))
            })
            .k_largest_relaxed_by_key(TOP_N, |(_, score)| *score)
            .map(|(path, _)| path)
            .collect();

        if baseline == streaming {
            println!("query {query:?}: identical top {TOP_N}");
        } else {
            let missing: Vec<_> = baseline
                .iter()
                .filter(|path| !streaming.contains(path))
                .collect();
            println!("query {query:?}: DIFFERS; baseline-only entries in top {TOP_N}: {missing:?}");
            println!("  baseline:  {baseline:?}");
            println!("  streaming: {streaming:?}");
        }
    }
}

/// Blocks until the initial filesystem scan finishes so the parity baseline
/// sees the complete candidate set.
fn wait_for_scan(session: &StreamingFileSearchSession) {
    let start = Instant::now();
    while session.lock_engine().is_scanning() && start.elapsed() < Duration::from_secs(60) {
        std::thread::sleep(Duration::from_millis(20));
    }
}
