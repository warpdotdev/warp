#![cfg(feature = "local_fs")]

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
use warpui::{App, EntityId};

use super::PaneGroupRepositoryRoots;
use crate::code::buffer_location::LocalOrRemotePath;
use crate::pane_group::WorkingDirectoriesModel;

fn local(path: &std::path::Path) -> LocalOrRemotePath {
    LocalOrRemotePath::Local(path.to_path_buf())
}

fn local_str(path: &str) -> LocalOrRemotePath {
    LocalOrRemotePath::Local(PathBuf::from(path))
}

#[test]
fn refresh_working_directories_collapses_subroots_to_nearest_repo_root() {
    App::test((), |mut app| async move {
        let detected_repos_handle = app.add_singleton_model(|_| DetectedRepositories::default());

        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let repo_a = repo_root.join("a");
        let repo_b = repo_root.join("b");
        fs::create_dir_all(&repo_a).expect("create repo/a");
        fs::create_dir_all(&repo_b).expect("create repo/b");

        // Use dunce::canonicalize to match the behavior of warp_util::standardized_path::StandardizedPath and normalize_cwd,
        // which strip the Windows extended-length path prefix (\\?\) for consistent comparison.
        let canonical_repo_root = dunce::canonicalize(&repo_root).expect("canonical repo root");

        // Seed DetectedRepositories so get_root_for_path resolves to this repo.
        detected_repos_handle.update(&mut app, |repos, _ctx| {
            let canonical =
                warp_util::standardized_path::StandardizedPath::from_local_canonicalized(
                    canonical_repo_root.as_path(),
                )
                .expect("canonicalized path");
            repos.insert_test_repo_root(canonical);
        });

        let pane_group_id = EntityId::new();
        let terminal_1 = EntityId::new();
        let terminal_2 = EntityId::new();

        let working_directories_handle = app.add_model(|_| WorkingDirectoriesModel::new());
        let roots: Vec<LocalOrRemotePath> =
            working_directories_handle.update(&mut app, |model, ctx| {
                model.refresh_working_directories_for_pane_group(
                    pane_group_id,
                    vec![
                        (terminal_1, LocalOrRemotePath::Local(repo_a.clone())),
                        (terminal_2, LocalOrRemotePath::Local(repo_b.clone())),
                    ],
                    vec![],
                    Some(terminal_1),
                    ctx,
                );

                model
                    .most_recent_directories_for_pane_group(pane_group_id)
                    .expect("pane group exists")
                    .map(|dir| dir.path)
                    .collect()
            });

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0], local(&canonical_repo_root));
    });
}

#[test]
fn refresh_working_directories_preserves_non_repo_paths_and_dedupes() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| DetectedRepositories::default());

        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let dir_1 = temp_dir.path().join("dir-1");
        let dir_2 = temp_dir.path().join("dir-2");
        fs::create_dir_all(&dir_1).expect("create dir-1");
        fs::create_dir_all(&dir_2).expect("create dir-2");

        // Use dunce::canonicalize to match the behavior of normalize_cwd,
        // which strips the Windows extended-length path prefix (\\?\) for consistent comparison.
        let canonical_1 = dunce::canonicalize(&dir_1).expect("canonical dir-1");
        let canonical_2 = dunce::canonicalize(&dir_2).expect("canonical dir-2");

        let pane_group_id = EntityId::new();
        let terminal_1 = EntityId::new();
        let terminal_2 = EntityId::new();
        let terminal_3 = EntityId::new();

        let working_directories_handle = app.add_model(|_| WorkingDirectoriesModel::new());
        let roots: HashSet<LocalOrRemotePath> =
            working_directories_handle.update(&mut app, |model, ctx| {
                model.refresh_working_directories_for_pane_group(
                    pane_group_id,
                    vec![
                        (terminal_1, LocalOrRemotePath::Local(dir_1.clone())),
                        (terminal_2, LocalOrRemotePath::Local(dir_2.clone())),
                        // Duplicate root should be deduped.
                        (terminal_3, LocalOrRemotePath::Local(dir_1.clone())),
                    ],
                    vec![],
                    Some(terminal_1),
                    ctx,
                );

                model
                    .most_recent_directories_for_pane_group(pane_group_id)
                    .expect("pane group exists")
                    .map(|dir| dir.path)
                    .collect()
            });

        assert_eq!(
            roots,
            HashSet::from_iter([local(&canonical_1), local(&canonical_2)]),
            "should preserve non-repo roots and dedupe exact paths"
        );
    });
}

// Regression test for GH-10598: the code review panel's manually selected
// repository must be remembered per pane group so it survives leaving and
// returning to an Agent session.
#[test]
fn selected_review_repo_is_remembered_per_pane_group() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| DetectedRepositories::default());

        let pane_group_a = EntityId::new();
        let pane_group_b = EntityId::new();
        let repo_x = PathBuf::from("/repos/x");
        let repo_y = PathBuf::from("/repos/y");
        let repo_p = PathBuf::from("/repos/p");

        let working_directories_handle = app.add_model(|_| WorkingDirectoriesModel::new());

        // Initially nothing is saved for either pane group.
        working_directories_handle.update(&mut app, |model, _ctx| {
            assert!(model.get_selected_review_repo(pane_group_a).is_none());
            assert!(model.get_selected_review_repo(pane_group_b).is_none());
        });

        // User selects repo Y in pane group A.
        working_directories_handle.update(&mut app, |model, _ctx| {
            model.set_selected_review_repo(pane_group_a, local(&repo_y));
        });

        // The selection for A is remembered and is independent from B's.
        working_directories_handle.update(&mut app, |model, _ctx| {
            assert_eq!(
                model.get_selected_review_repo(pane_group_a).cloned(),
                Some(local(&repo_y)),
                "pane group A should remember its manual selection"
            );
            assert!(
                model.get_selected_review_repo(pane_group_b).is_none(),
                "pane group B should be untouched by selections in A"
            );
        });

        // User selects repo P in pane group B; A's selection must not change.
        working_directories_handle.update(&mut app, |model, _ctx| {
            model.set_selected_review_repo(pane_group_b, local(&repo_p));
            assert_eq!(
                model.get_selected_review_repo(pane_group_a).cloned(),
                Some(local(&repo_y)),
                "selecting in B must not clobber A's saved selection"
            );
            assert_eq!(
                model.get_selected_review_repo(pane_group_b).cloned(),
                Some(local(&repo_p)),
            );
        });

        // Updating A's selection overwrites the previous saved value for A.
        working_directories_handle.update(&mut app, |model, _ctx| {
            model.set_selected_review_repo(pane_group_a, local(&repo_x));
            assert_eq!(
                model.get_selected_review_repo(pane_group_a).cloned(),
                Some(local(&repo_x)),
            );
        });
    });
}

// Regression test for GH-10598: closing a tab (i.e. destroying a pane group)
// must clean up the saved code-review-panel selection so it cannot leak into
// or be confused with a future pane group that happens to reuse an EntityId.
#[test]
fn selected_review_repo_is_cleared_when_pane_group_is_removed() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| DetectedRepositories::default());

        let pane_group_id = EntityId::new();
        let repo = PathBuf::from("/repos/x");

        let working_directories_handle = app.add_model(|_| WorkingDirectoriesModel::new());

        working_directories_handle.update(&mut app, |model, ctx| {
            model.set_selected_review_repo(pane_group_id, local(&repo));
            assert_eq!(
                model.get_selected_review_repo(pane_group_id).cloned(),
                Some(local(&repo)),
            );

            model.remove_pane_group(pane_group_id, ctx);
            assert!(
                model.get_selected_review_repo(pane_group_id).is_none(),
                "removing a pane group must clear its saved review-panel selection"
            );
        });
    });
}

// ── PaneGroupRepositoryRoots unit tests ──────────────────────────

#[test]
fn pane_group_repository_roots_insert_updates_both_maps() {
    let mut roots = PaneGroupRepositoryRoots::default();
    let pane_a = EntityId::new();
    let path = local_str("/repos/x");

    assert!(roots.insert(pane_a, path.clone()));
    // Re-inserting the same (pane_group, path) is a no-op.
    assert!(!roots.insert(pane_a, path.clone()));

    let forward = roots.get(pane_a).expect("pane group registered");
    assert!(forward.contains(&path), "forward map must contain the path");
    assert_eq!(
        roots.path_to_pane_groups.get(&path).cloned(),
        Some(HashSet::from_iter([pane_a])),
        "reverse map must reflect the inserted pane group"
    );
}

#[test]
fn pane_group_repository_roots_set_paths_returns_only_truly_orphaned_paths() {
    let mut roots = PaneGroupRepositoryRoots::default();
    let pane_a = EntityId::new();
    let pane_b = EntityId::new();
    let shared = local_str("/repos/shared");
    let only_a = local_str("/repos/only-a");

    // Both pane groups reference `shared`; only A references `only_a`.
    let orphans_a = roots.set_paths(pane_a, vec![shared.clone(), only_a.clone()]);
    assert!(orphans_a.is_empty(), "first insert never produces orphans");
    let orphans_b = roots.set_paths(pane_b, vec![shared.clone()]);
    assert!(orphans_b.is_empty());

    // A drops both of its paths.
    let orphans = roots.set_paths(pane_a, Vec::<LocalOrRemotePath>::new());

    // `shared` is still referenced by B, so it must not be reported as orphaned.
    // `only_a` was only referenced by A, so it must be.
    assert_eq!(
        orphans,
        vec![only_a.clone()],
        "shared paths must not appear in the orphan list"
    );

    // Reverse map: `shared` only references B; `only_a` is gone entirely.
    assert_eq!(
        roots.path_to_pane_groups.get(&shared).cloned(),
        Some(HashSet::from_iter([pane_b])),
    );
    assert!(
        !roots.path_to_pane_groups.contains_key(&only_a),
        "orphaned path must be evicted from the reverse map"
    );

    // Forward map: A is now empty (entry retained), B still owns `shared`.
    let a_forward = roots.get(pane_a).expect("pane group A entry retained");
    assert!(a_forward.is_empty(), "A's forward set should be empty");
    let b_forward = roots.get(pane_b).expect("pane group B entry retained");
    assert!(b_forward.contains(&shared));
}

#[test]
fn pane_group_repository_roots_set_paths_preserves_insertion_order() {
    let mut roots = PaneGroupRepositoryRoots::default();
    let pane = EntityId::new();
    let x = local_str("/repos/x");
    let y = local_str("/repos/y");
    let z = local_str("/repos/z");

    // Initial set in order x, y.
    let _ = roots.set_paths(pane, vec![x.clone(), y.clone()]);

    // Replace with y, z. y should keep its position; z is appended; x is removed.
    let _ = roots.set_paths(pane, vec![y.clone(), z.clone()]);

    let forward: Vec<LocalOrRemotePath> = roots
        .get(pane)
        .expect("pane group present")
        .iter()
        .cloned()
        .collect();
    assert_eq!(
        forward,
        vec![y, z],
        "existing items must keep their order; new items appended after"
    );
}

#[test]
fn pane_group_repository_roots_remove_pane_group_returns_orphans() {
    let mut roots = PaneGroupRepositoryRoots::default();
    let pane_a = EntityId::new();
    let pane_b = EntityId::new();
    let shared = local_str("/repos/shared");
    let only_a = local_str("/repos/only-a");

    let _ = roots.set_paths(pane_a, vec![shared.clone(), only_a.clone()]);
    let _ = roots.set_paths(pane_b, vec![shared.clone()]);

    // Removing A while B still references `shared` only orphans `only_a`.
    let orphans: HashSet<LocalOrRemotePath> = roots
        .remove_pane_group(pane_a)
        .expect("pane group A was present")
        .into_iter()
        .collect();
    assert_eq!(orphans, HashSet::from_iter([only_a.clone()]));
    assert!(roots.get(pane_a).is_none(), "pane group A entry is gone");

    // Removing B now orphans `shared`.
    let orphans = roots
        .remove_pane_group(pane_b)
        .expect("pane group B was present");
    assert_eq!(orphans, vec![shared.clone()]);
    assert!(roots.path_to_pane_groups.is_empty());
    assert!(roots.pane_group_to_paths.is_empty());
}

#[test]
fn pane_group_repository_roots_remove_unknown_pane_group_is_noop() {
    let mut roots = PaneGroupRepositoryRoots::default();
    let missing = EntityId::new();
    assert!(roots.remove_pane_group(missing).is_none());
}

// ── End-to-end cleanup behavior tests ────────────────────────────

/// Helper for end-to-end cleanup tests: registers the singletons required by
/// `DiffStateModel::new_local` (the `DirectoryWatcher`), prepares a temp dir,
/// seeds it as a detected repo root, and returns the canonical repo path along
/// with a fresh `WorkingDirectoriesModel` handle.
fn setup_repo(
    app: &mut warpui::App,
    detected_repos: &warpui::ModelHandle<DetectedRepositories>,
) -> (
    tempfile::TempDir,
    PathBuf,
    PathBuf,
    warpui::ModelHandle<WorkingDirectoriesModel>,
) {
    app.add_singleton_model(DirectoryWatcher::new_for_testing);

    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let repo_path = temp_dir.path().join("repo");
    fs::create_dir_all(&repo_path).expect("create repo dir");
    let canonical_repo = dunce::canonicalize(&repo_path).expect("canonical repo");

    detected_repos.update(app, |repos, _ctx| {
        let canonical = warp_util::standardized_path::StandardizedPath::from_local_canonicalized(
            canonical_repo.as_path(),
        )
        .expect("canonicalized path");
        repos.insert_test_repo_root(canonical);
    });

    let working_directories_handle = app.add_model(|_| WorkingDirectoriesModel::new());
    (
        temp_dir,
        repo_path,
        canonical_repo,
        working_directories_handle,
    )
}

/// Regression: closing pane group A while pane group B still references the
/// same repo must NOT drop the shared `DiffStateModel`. Before the fix,
/// `drop_unused_diff_state_models` removed the cache entry unconditionally for
/// any repo that left A's set, even when B still relied on it.
#[test]
fn shared_diff_state_model_survives_when_other_pane_group_still_references_repo() {
    App::test((), |mut app| async move {
        let detected_repos = app.add_singleton_model(|_| DetectedRepositories::default());

        let pane_group_a = EntityId::new();
        let pane_group_b = EntityId::new();
        let terminal_a = EntityId::new();
        let terminal_b = EntityId::new();

        let (_temp_dir, repo_path, canonical_repo, working_directories_handle) =
            setup_repo(&mut app, &detected_repos);

        // Both pane groups land in the same repo.
        working_directories_handle.update(&mut app, |model, ctx| {
            model.refresh_working_directories_for_pane_group(
                pane_group_a,
                vec![(terminal_a, LocalOrRemotePath::Local(repo_path.clone()))],
                vec![],
                Some(terminal_a),
                ctx,
            );
            model.refresh_working_directories_for_pane_group(
                pane_group_b,
                vec![(terminal_b, LocalOrRemotePath::Local(repo_path.clone()))],
                vec![],
                Some(terminal_b),
                ctx,
            );
        });

        // Open the shared diff state model.
        let initial_id = working_directories_handle.update(&mut app, |model, ctx| {
            model
                .get_or_create_diff_state_model(local(&canonical_repo), ctx)
                .expect("local diff state model must be created")
                .id()
        });

        // Pane group A's terminals go away (close the tab path).
        working_directories_handle.update(&mut app, |model, ctx| {
            model.refresh_working_directories_for_pane_group(
                pane_group_a,
                vec![],
                vec![],
                None,
                ctx,
            );
        });

        // Re-fetching should return the SAME cached model (no re-creation).
        let after_id = working_directories_handle.update(&mut app, |model, ctx| {
            model
                .get_or_create_diff_state_model(local(&canonical_repo), ctx)
                .expect("local diff state model must still be present")
                .id()
        });

        assert_eq!(
            initial_id, after_id,
            "shared DiffStateModel must survive when another pane group still references the repo"
        );
    });
}

/// When the last pane group referencing a repo navigates away, the shared
/// `DiffStateModel` is dropped from the cache, so a subsequent
/// `get_or_create_diff_state_model` creates a fresh model.
#[test]
fn diff_state_model_is_dropped_when_no_pane_group_references_repo() {
    App::test((), |mut app| async move {
        let detected_repos = app.add_singleton_model(|_| DetectedRepositories::default());

        let pane_group = EntityId::new();
        let terminal = EntityId::new();

        let (_temp_dir, repo_path, canonical_repo, working_directories_handle) =
            setup_repo(&mut app, &detected_repos);

        working_directories_handle.update(&mut app, |model, ctx| {
            model.refresh_working_directories_for_pane_group(
                pane_group,
                vec![(terminal, LocalOrRemotePath::Local(repo_path.clone()))],
                vec![],
                Some(terminal),
                ctx,
            );
        });

        let initial_id = working_directories_handle.update(&mut app, |model, ctx| {
            model
                .get_or_create_diff_state_model(local(&canonical_repo), ctx)
                .expect("local diff state model must be created")
                .id()
        });

        // Only pane group leaves the repo → model is orphaned and dropped.
        working_directories_handle.update(&mut app, |model, ctx| {
            model.refresh_working_directories_for_pane_group(pane_group, vec![], vec![], None, ctx);
        });

        let after_id = working_directories_handle.update(&mut app, |model, ctx| {
            model
                .get_or_create_diff_state_model(local(&canonical_repo), ctx)
                .expect("local diff state model must be re-created")
                .id()
        });

        assert_ne!(
            initial_id, after_id,
            "DiffStateModel should be dropped and re-created when no pane group references the repo"
        );
    });
}

/// `remove_pane_group` (explicit tab teardown) must respect the same refcount
/// semantics: pane group B's shared `DiffStateModel` survives when A is closed.
#[test]
fn remove_pane_group_does_not_drop_diff_state_model_shared_with_other_pane_group() {
    App::test((), |mut app| async move {
        let detected_repos = app.add_singleton_model(|_| DetectedRepositories::default());

        let pane_group_a = EntityId::new();
        let pane_group_b = EntityId::new();
        let terminal_a = EntityId::new();
        let terminal_b = EntityId::new();

        let (_temp_dir, repo_path, canonical_repo, working_directories_handle) =
            setup_repo(&mut app, &detected_repos);

        working_directories_handle.update(&mut app, |model, ctx| {
            model.refresh_working_directories_for_pane_group(
                pane_group_a,
                vec![(terminal_a, LocalOrRemotePath::Local(repo_path.clone()))],
                vec![],
                Some(terminal_a),
                ctx,
            );
            model.refresh_working_directories_for_pane_group(
                pane_group_b,
                vec![(terminal_b, LocalOrRemotePath::Local(repo_path.clone()))],
                vec![],
                Some(terminal_b),
                ctx,
            );
        });

        let initial_id = working_directories_handle.update(&mut app, |model, ctx| {
            model
                .get_or_create_diff_state_model(local(&canonical_repo), ctx)
                .expect("local diff state model must be created")
                .id()
        });

        // Tear down pane group A.
        working_directories_handle.update(&mut app, |model, ctx| {
            model.remove_pane_group(pane_group_a, ctx);
        });

        let after_id = working_directories_handle.update(&mut app, |model, ctx| {
            model
                .get_or_create_diff_state_model(local(&canonical_repo), ctx)
                .expect("local diff state model must still be present")
                .id()
        });

        assert_eq!(
            initial_id, after_id,
            "removing pane group A must not drop a model that pane group B still references"
        );
    });
}

#[test]
fn clear_selected_review_repo_removes_only_the_targeted_pane_group_entry() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| DetectedRepositories::default());

        let pane_group_a = EntityId::new();
        let pane_group_b = EntityId::new();
        let repo_a = PathBuf::from("/repos/a");
        let repo_b = PathBuf::from("/repos/b");

        let working_directories_handle = app.add_model(|_| WorkingDirectoriesModel::new());

        working_directories_handle.update(&mut app, |model, _ctx| {
            model.set_selected_review_repo(pane_group_a, local(&repo_a));
            model.set_selected_review_repo(pane_group_b, local(&repo_b));

            model.clear_selected_review_repo(pane_group_a);

            assert!(model.get_selected_review_repo(pane_group_a).is_none());
            assert_eq!(
                model.get_selected_review_repo(pane_group_b).cloned(),
                Some(local(&repo_b)),
            );
        });
    });
}
