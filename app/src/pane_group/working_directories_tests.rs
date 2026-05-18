#![cfg(feature = "local_fs")]

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use crate::code::buffer_location::LocalOrRemotePath;
use repo_metadata::repositories::DetectedRepositories;
use warpui::{App, EntityId};

use crate::pane_group::WorkingDirectoriesModel;

fn local(path: &std::path::Path) -> LocalOrRemotePath {
    LocalOrRemotePath::Local(path.to_path_buf())
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
        let roots: Vec<PathBuf> = working_directories_handle.update(&mut app, |model, ctx| {
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
        assert_eq!(roots[0], canonical_repo_root);
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
        let roots: HashSet<PathBuf> = working_directories_handle.update(&mut app, |model, ctx| {
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
            HashSet::from_iter([canonical_1, canonical_2]),
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
