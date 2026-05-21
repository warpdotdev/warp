use std::fs;

use crate::repositories::{stub_git_repository, RepoDetectionSource};
use crate::{repositories::DetectedRepositories, watcher::DirectoryWatcher};
use virtual_fs::{Stub, VirtualFS};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::App;

#[test]
fn test_detect_possible_local_git_repo_non_existent_directory() {
    VirtualFS::test("detect_non_existent", |dirs, _vfs| {
        let non_existent_path = dirs.tests().join("non_existent_directory");

        App::test((), |mut app| async move {
            let _watcher = app.add_model(DirectoryWatcher::new);
            let repo_handle = app.add_model(|_| DetectedRepositories::default());

            repo_handle.update(&mut app, |watcher, ctx| {
                std::mem::drop(watcher.detect_possible_local_git_repo(
                    &non_existent_path.to_string_lossy(),
                    RepoDetectionSource::TerminalNavigation,
                    ctx,
                ));
            });

            // Since the path doesn't exist, canonicalization fails and so there's no spawned future.
            repo_handle.read(&app, |repos, _| {
                assert!(repos.spawned_futures().is_empty());
                assert!(repos.repository_roots.is_empty());
            });
        });
    });
}

#[test]
fn test_detect_possible_local_git_repo_not_a_git_repo() {
    VirtualFS::test("detect_not_git", |dirs, mut vfs| {
        // Create a regular directory structure without .git
        vfs.mkdir("regular_dir/subdir").with_files(vec![
            Stub::FileWithContent("regular_dir/file1.txt", "content1"),
            Stub::FileWithContent("regular_dir/subdir/file2.txt", "content2"),
        ]);

        let regular_dir = dirs.tests().join("regular_dir");

        App::test((), |mut app| async move {
            let _watcher = app.add_model(DirectoryWatcher::new);
            let repo_handle = app.add_model(|_| DetectedRepositories::default());

            repo_handle.update(&mut app, |watcher, ctx| {
                std::mem::drop(watcher.detect_possible_local_git_repo(
                    &regular_dir.to_string_lossy(),
                    RepoDetectionSource::TerminalNavigation,
                    ctx,
                ));
            });

            repo_handle
                .update(&mut app, |watcher, ctx| {
                    let future_id = watcher.spawned_futures()[0];
                    ctx.await_spawned_future(future_id)
                })
                .await;

            // Since no git repo is found, no repository should be registered.
            let regular_canonical =
                StandardizedPath::from_local_canonicalized(&regular_dir).unwrap();
            repo_handle.read(&app, |watcher, _ctx| {
                let key = LocalOrRemotePath::Local(regular_canonical.to_local_path().unwrap());
                assert!(!watcher.repository_roots.contains(&key));
            });
        });
    });
}

#[test]
fn test_detect_possible_local_git_repo_nested_repo_created_after_parent_registration() {
    VirtualFS::test("detect_nested_repo", |dirs, mut vfs| {
        // Create a parent git repository structure
        stub_git_repository(&mut vfs, "parent_repo");

        let parent_repo = dirs.tests().join("parent_repo");
        let nested_project = parent_repo.join("projects/nested_project");
        let parent_canonical_path =
            StandardizedPath::from_local_canonicalized(&parent_repo).unwrap();

        App::test((), |mut app| async move {
            let watcher_handle = app.add_singleton_model(DirectoryWatcher::new);
            let repo_handle = app.add_model(|_| DetectedRepositories::default());

            // First, register the parent repository
            // Now, try to detect the nested git repo.
            repo_handle
                .update(&mut app, |repo, ctx| {
                    std::mem::drop(repo.detect_possible_local_git_repo(
                        &parent_repo.to_string_lossy(),
                        RepoDetectionSource::TerminalNavigation,
                        ctx,
                    ));
                    let future_id = repo.spawned_futures().last().unwrap();
                    ctx.await_spawned_future(*future_id)
                })
                .await;

            // Verify parent is registered
            repo_handle.read(&app, |repo, _ctx| {
                assert!(repo
                    .get_root_for_path(&LocalOrRemotePath::Local(
                        parent_canonical_path.to_local_path().unwrap(),
                    ))
                    .is_some());
            });

            // Now simulate creating a nested git repo in the projects/nested_project directory.
            // This would happen in real life when someone runs `git init` in a subdirectory.
            stub_git_repository(&mut vfs, "parent_repo/projects/nested_project");

            // Now, try to detect the nested git repo.
            repo_handle
                .update(&mut app, |repo, ctx| {
                    std::mem::drop(repo.detect_possible_local_git_repo(
                        &nested_project.to_string_lossy(),
                        RepoDetectionSource::TerminalNavigation,
                        ctx,
                    ));
                    let future_id = repo.spawned_futures().last().unwrap();
                    ctx.await_spawned_future(*future_id)
                })
                .await;

            // Verify that both repositories are now registered.
            let nested_canonical_path =
                StandardizedPath::from_local_canonicalized(&nested_project).unwrap();
            repo_handle.read(&app, |repo, _ctx| {
                // Parent should still be registered
                assert!(repo
                    .get_root_for_path(&LocalOrRemotePath::Local(
                        parent_canonical_path.to_local_path().unwrap(),
                    ))
                    .is_some());
                // Nested project should now also be registered as its own repo
                assert!(repo
                    .get_root_for_path(&LocalOrRemotePath::Local(
                        nested_canonical_path.to_local_path().unwrap(),
                    ))
                    .is_some());
            });

            // Check a subdirectory of the nested repo. This path must exist for canonicalization to succeed.
            let nested_project_contents = nested_project.join("src");
            fs::create_dir(&nested_project_contents).expect("Creating nested directory failed");

            // Verify that querying from the nested directory returns the nested repo, not the parent
            watcher_handle.read(&app, |watcher, ctx| {
                let found_handle = watcher.get_watched_directory_for_path(&nested_project_contents);
                assert!(found_handle.is_some());

                assert_eq!(
                    found_handle.unwrap().as_ref(ctx).root_dir(),
                    &nested_canonical_path
                );
            });
        });
    });
}

#[test]
#[cfg(feature = "local_fs")]
fn test_find_git_repo_with_worktree() {
    VirtualFS::test("find_git_repo_worktree", |dirs, mut vfs| {
        // Set up a primary repository with a worktree directory.
        vfs.mkdir("main_repo/.git/objects");
        vfs.mkdir("main_repo/.git/worktrees/wt1");
        vfs.with_files(vec![
            Stub::FileWithContent("main_repo/.git/HEAD", "ref: refs/heads/main"),
            // Worktree-specific gitdir contains a HEAD file
            Stub::FileWithContent("main_repo/.git/worktrees/wt1/HEAD", "ref: refs/heads/main"),
        ]);

        // Set up the worktree checkout directory with a gitfile pointing to the worktree gitdir.
        vfs.mkdir("checkout_wt1/src");
        vfs.with_files(vec![Stub::FileWithContent(
            "checkout_wt1/.git",
            "gitdir: ../main_repo/.git/worktrees/wt1",
        )]);

        let worktree_root = dirs.tests().join("checkout_wt1");
        let expected_gitdir = dirs.tests().join("main_repo/.git/worktrees/wt1");

        App::test((), |mut _app| async move {
            let result = super::find_git_repo(worktree_root.as_path()).await;
            assert!(result.is_some(), "expected to find a git repo for worktree");
            let info = result.unwrap();

            // The working tree path should be the worktree checkout directory
            assert_eq!(
                info.working_tree_path.as_deref(),
                Some(worktree_root.as_path())
            );

            // The git_dir_path should resolve to the worktree's gitdir inside the primary repo.
            let actual = std::fs::canonicalize(&info.git_dir_path).expect("canonicalize gitdir");
            let expected = std::fs::canonicalize(&expected_gitdir).expect("canonicalize expected");
            assert_eq!(actual, expected);
        });
    });
}
