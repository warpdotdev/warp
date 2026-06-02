//! Unit tests for the pure repo-selection decision in [`super`] (the view
//! layer). These cover [`pick_selected_repo`], which decides which discovered
//! repo a tab lands on — the logic that keeps a per-tab repo choice across tab
//! switches.

use super::*;

/// Three sibling repos under a common parent; the anchor lives inside the
/// first one. `pick_selected_repo` matches a repo by `anchor.starts_with(r)`,
/// so the paths share a prefix on purpose.
fn repos() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/work/repo1"),
        PathBuf::from("/work/repo2"),
        PathBuf::from("/work/repo3"),
    ]
}

#[test]
fn follow_anchor_without_saved_selects_repo_containing_anchor() {
    // Fresh tab, no prior manual pick: follow the anchor to the repo it's in.
    let repos = repos();
    let selected = pick_selected_repo(
        &repos,
        Path::new("/work/repo1/src"),
        true,
        None,
        None,
    );
    assert_eq!(selected, Some(0));
}

#[test]
fn follow_anchor_restores_saved_selection_over_anchor_repo() {
    // The reported bug: the anchor lives in repo1, but the user had manually
    // picked repo3 for this tab. Following the anchor must NOT snap back to
    // repo1 — the saved choice wins.
    let repos = repos();
    let saved = PathBuf::from("/work/repo3");
    let selected = pick_selected_repo(
        &repos,
        Path::new("/work/repo1/src"),
        true,
        None,
        Some(&saved),
    );
    assert_eq!(selected, Some(2));
}

#[test]
fn follow_anchor_falls_back_to_anchor_when_saved_repo_gone() {
    // The saved repo no longer exists in the discovered list (e.g. deleted):
    // fall through to following the anchor.
    let repos = repos();
    let saved = PathBuf::from("/work/removed");
    let selected = pick_selected_repo(
        &repos,
        Path::new("/work/repo2/deep/dir"),
        true,
        None,
        Some(&saved),
    );
    assert_eq!(selected, Some(1));
}

#[test]
fn follow_anchor_keeps_previous_when_anchor_outside_all_repos() {
    // Anchor isn't inside any discovered repo and nothing is saved: keep the
    // previously selected repo rather than jumping to the first.
    let repos = repos();
    let previous = PathBuf::from("/work/repo3");
    let selected = pick_selected_repo(
        &repos,
        Path::new("/elsewhere"),
        true,
        Some(&previous),
        None,
    );
    assert_eq!(selected, Some(2));
}

#[test]
fn follow_anchor_falls_back_to_first_when_nothing_matches() {
    let repos = repos();
    let selected = pick_selected_repo(&repos, Path::new("/elsewhere"), true, None, None);
    assert_eq!(selected, Some(0));
}

#[test]
fn no_follow_keeps_previous_and_ignores_saved_and_anchor() {
    // Refresh / scan-depth change: keep the current repo. The saved selection
    // and the anchor's own repo are both irrelevant here.
    let repos = repos();
    let previous = PathBuf::from("/work/repo2");
    let saved = PathBuf::from("/work/repo3");
    let selected = pick_selected_repo(
        &repos,
        Path::new("/work/repo1/src"),
        false,
        Some(&previous),
        Some(&saved),
    );
    assert_eq!(selected, Some(1));
}

#[test]
fn no_follow_falls_back_to_first_when_previous_gone() {
    let repos = repos();
    let previous = PathBuf::from("/work/removed");
    let selected = pick_selected_repo(&repos, Path::new("/work/repo1"), false, Some(&previous), None);
    assert_eq!(selected, Some(0));
}

#[test]
fn empty_repo_list_selects_nothing() {
    let repos: Vec<PathBuf> = Vec::new();
    assert_eq!(
        pick_selected_repo(&repos, Path::new("/work/repo1"), true, None, None),
        None
    );
    assert_eq!(
        pick_selected_repo(&repos, Path::new("/work/repo1"), false, None, None),
        None
    );
}
