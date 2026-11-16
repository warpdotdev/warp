use std::collections::HashMap;
use std::path::{Path, PathBuf};

use warp_core::ui::theme::AnsiColorIdentifier;

use super::compute_candidate_paths;
use crate::workspace::tab_settings::{DirectoryTabColor, DirectoryTabColors};

fn colors(entries: &[(&str, DirectoryTabColor)]) -> DirectoryTabColors {
    let map: HashMap<String, DirectoryTabColor> = entries
        .iter()
        .map(|(path, color)| ((*path).to_string(), *color))
        .collect();
    DirectoryTabColors(map)
}

fn all_exist(_: &Path) -> bool {
    true
}

#[test]
fn test_union_dedupes_across_sources() {
    let indexed = vec![PathBuf::from("/nonexistent/repo_a")];
    let persisted = vec![PathBuf::from("/nonexistent/repo_a")];
    let existing = DirectoryTabColors::default();

    let candidates = compute_candidate_paths(indexed, persisted, &existing, all_exist);

    assert_eq!(candidates, vec![PathBuf::from("/nonexistent/repo_a")]);
}

#[test]
fn test_filters_out_existing_non_suppressed_entries() {
    let indexed = vec![
        PathBuf::from("/nonexistent/unassigned"),
        PathBuf::from("/nonexistent/colored"),
        PathBuf::from("/nonexistent/fresh"),
    ];
    let existing = colors(&[
        ("/nonexistent/unassigned", DirectoryTabColor::Unassigned),
        (
            "/nonexistent/colored",
            DirectoryTabColor::Color(AnsiColorIdentifier::Red),
        ),
    ]);

    let candidates = compute_candidate_paths(indexed, Vec::<PathBuf>::new(), &existing, all_exist);

    assert_eq!(candidates, vec![PathBuf::from("/nonexistent/fresh")]);
}

#[test]
fn test_retains_suppressed_entries_as_candidates() {
    let indexed = vec![PathBuf::from("/nonexistent/suppressed_repo")];
    let existing = colors(&[(
        "/nonexistent/suppressed_repo",
        DirectoryTabColor::Suppressed,
    )]);

    let candidates = compute_candidate_paths(indexed, Vec::<PathBuf>::new(), &existing, all_exist);

    assert_eq!(
        candidates,
        vec![PathBuf::from("/nonexistent/suppressed_repo")]
    );
}

#[test]
fn test_non_existent_paths_are_dropped() {
    let indexed = vec![
        PathBuf::from("/nonexistent/a"),
        PathBuf::from("/nonexistent/b"),
    ];
    let existing = DirectoryTabColors::default();

    let candidates = compute_candidate_paths(indexed, Vec::<PathBuf>::new(), &existing, |p| {
        p == Path::new("/nonexistent/b")
    });

    assert_eq!(candidates, vec![PathBuf::from("/nonexistent/b")]);
}

#[test]
fn test_worktree_paths_are_kept() {
    let indexed = vec![
        PathBuf::from("/users/alice/.warp-dev/worktrees/warp-internal/feature_a"),
        PathBuf::from("/users/alice/.warp-dev/worktrees/warp-internal/feature_b"),
        PathBuf::from("/users/alice/code/primary-repo"),
    ];
    let existing = DirectoryTabColors::default();

    let candidates = compute_candidate_paths(indexed, Vec::<PathBuf>::new(), &existing, all_exist);

    assert_eq!(
        candidates,
        vec![
            PathBuf::from("/users/alice/.warp-dev/worktrees/warp-internal/feature_a"),
            PathBuf::from("/users/alice/.warp-dev/worktrees/warp-internal/feature_b"),
            PathBuf::from("/users/alice/code/primary-repo"),
        ]
    );
}

#[test]
fn test_results_are_sorted_alphabetically_by_canonical_key() {
    let indexed = vec![
        PathBuf::from("/nonexistent/zulu"),
        PathBuf::from("/nonexistent/alpha"),
    ];
    let persisted = vec![PathBuf::from("/nonexistent/mango")];
    let existing = DirectoryTabColors::default();

    let candidates = compute_candidate_paths(indexed, persisted, &existing, all_exist);

    assert_eq!(
        candidates,
        vec![
            PathBuf::from("/nonexistent/alpha"),
            PathBuf::from("/nonexistent/mango"),
            PathBuf::from("/nonexistent/zulu"),
        ]
    );
}

#[test]
fn test_empty_inputs_produce_empty_output() {
    let existing = DirectoryTabColors::default();

    let candidates = compute_candidate_paths(
        Vec::<PathBuf>::new(),
        Vec::<PathBuf>::new(),
        &existing,
        all_exist,
    );

    assert!(candidates.is_empty());
}
