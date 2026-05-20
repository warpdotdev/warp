use std::fs;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::repositories::stub_git_repository;
use crate::repository::{RepositorySubscriber, TrackedRemoteRef};
use crate::watcher::{DirectoryWatcher, TaskQueue};
use crate::{CanonicalizedPath, RepoMetadataError, Repository, RepositoryUpdate};
use futures::channel::mpsc;
use futures::StreamExt as _;
use virtual_fs::{Stub, VirtualFS};
use warp_util::standardized_path::StandardizedPath;
use warpui::r#async::Timer;
use warpui::{App, ModelContext, ModelHandle};

#[test]
fn test_add_repository_success() {
    VirtualFS::test("add_repo_success", |dirs, mut vfs| {
        stub_git_repository(&mut vfs, "git_repo");

        let git_repo = dirs.tests().join("git_repo");
        let canonical_path = StandardizedPath::from_local_canonicalized(&git_repo).unwrap();

        App::test((), |mut app| async move {
            let watcher_handle = app.add_model(DirectoryWatcher::new);

            let result = watcher_handle.update(&mut app, |watcher, ctx| {
                watcher.add_directory(canonical_path.clone(), ctx)
            });

            assert!(result.is_ok());
            let repo_handle = result.unwrap();

            // Verify the repository was registered
            let repo_path = repo_handle.read(&app, |repo, _ctx| repo.root_dir().clone());
            assert_eq!(repo_path.to_local_path().as_deref().unwrap(), git_repo);

            // Verify it's in the watcher's registry
            watcher_handle.read(&app, |watcher, _ctx| {
                assert!(watcher.is_directory_watched(&canonical_path));
            });
        });
    });
}

#[test]
fn test_add_repository_non_existent() {
    VirtualFS::test("add_repo_nonexistent", |dirs, _vfs| {
        let non_existent = dirs.tests().join("non_existent");
        // Don't create the directory

        App::test((), |mut app| async move {
            let watcher_handle = app.add_model(DirectoryWatcher::new);

            let result = watcher_handle.update(&mut app, |watcher, ctx| {
                // This will fail because we can't canonicalize a non-existent path
                let canonical_result = CanonicalizedPath::try_from(&non_existent);
                match canonical_result {
                    Ok(canonical_path) => watcher.add_directory(canonical_path.into(), ctx),
                    Err(_) => Err(RepoMetadataError::RepoNotFound(
                        "Path does not exist".to_string(),
                    )),
                }
            });

            assert!(result.is_err());
            match result.unwrap_err() {
                RepoMetadataError::RepoNotFound(_) => {} // Expected
                _ => panic!("Expected RepoNotFound error"),
            }
        });
    });
}

/// A test subscriber that uses async channels to signal scan completion.
struct TestSubscriber {
    scan_completed_tx: mpsc::UnboundedSender<()>,
    update_completed_tx: mpsc::UnboundedSender<RepositoryUpdate>,
    active_tasks: Arc<AtomicUsize>,
}

impl TestSubscriber {
    fn new(
        scan_completed_tx: mpsc::UnboundedSender<()>,
        update_completed_tx: mpsc::UnboundedSender<RepositoryUpdate>,
        active_scans: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            scan_completed_tx,
            update_completed_tx,
            active_tasks: active_scans,
        }
    }
}

impl RepositorySubscriber for TestSubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let scan_completed_tx = self.scan_completed_tx.clone();
        let active_tasks = self.active_tasks.clone();

        Box::pin(async move {
            let prev = active_tasks.fetch_add(1, Ordering::SeqCst);
            // Ensure we never exceed the allowed concurrency.
            assert!(
                prev < super::MAX_CONCURRENT_TASKS,
                "Started a scan while already at concurrency limit"
            );

            // Simulate scan work.
            Timer::after(Duration::from_millis(10)).await;

            active_tasks.fetch_sub(1, Ordering::SeqCst);
            let _ = scan_completed_tx.unbounded_send(());
        })
    }

    fn on_files_updated(
        &mut self,
        _repository: &Repository,
        update: &RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let update = update.clone();
        let update_completed_tx = self.update_completed_tx.clone();
        let active_tasks = self.active_tasks.clone();

        Box::pin(async move {
            let prev = active_tasks.fetch_add(1, Ordering::SeqCst);
            // Ensure we never exceed the allowed concurrency.
            assert!(
                prev < super::MAX_CONCURRENT_TASKS,
                "Started an incremental update while already at concurrency limit"
            );

            // Simulate update work.
            Timer::after(Duration::from_millis(10)).await;

            active_tasks.fetch_sub(1, Ordering::SeqCst);
            let _ = update_completed_tx.unbounded_send(update);
        })
    }
}

#[test]
#[ignore = "flaky test: CODE-1748"]
fn test_task_queue_processes_all_tasks() {
    VirtualFS::test("task_queue_processes_all_tasks", |dirs, mut vfs| {
        stub_git_repository(&mut vfs, "repo1");
        stub_git_repository(&mut vfs, "repo2");

        let repo1 = dirs.tests().join("repo1");
        let repo2 = dirs.tests().join("repo2");

        App::test((), |mut app| async move {
            let watcher_handle = app.add_singleton_model(DirectoryWatcher::new);

            let repo1_handle = watcher_handle
                .update(&mut app, |watcher, ctx| {
                    watcher.add_directory(
                        StandardizedPath::from_local_canonicalized(&repo1).unwrap(),
                        ctx,
                    )
                })
                .unwrap();

            let repo2_handle = watcher_handle
                .update(&mut app, |watcher, ctx| {
                    watcher.add_directory(
                        StandardizedPath::from_local_canonicalized(&repo2).unwrap(),
                        ctx,
                    )
                })
                .unwrap();

            let active_tasks = Arc::new(AtomicUsize::new(0));
            let (scan_tx, mut scan_rx) = mpsc::unbounded::<()>();
            let (update_tx, _) = mpsc::unbounded::<RepositoryUpdate>();

            // Spawn 10 watchers per repository
            for _ in 0..10 {
                let subscriber1 =
                    TestSubscriber::new(scan_tx.clone(), update_tx.clone(), active_tasks.clone());
                let subscriber2 =
                    TestSubscriber::new(scan_tx.clone(), update_tx.clone(), active_tasks.clone());

                let start = repo1_handle.update(&mut app, |repo, ctx| {
                    repo.start_watching(Box::new(subscriber1), ctx)
                });
                start
                    .registration_future
                    .await
                    .expect("Failed to add subscriber");

                let start = repo2_handle.update(&mut app, |repo, ctx| {
                    repo.start_watching(Box::new(subscriber2), ctx)
                });
                start
                    .registration_future
                    .await
                    .expect("Failed to add subscriber");
            }

            // Wait for all scans to complete by consuming all messages
            for _ in 0..20 {
                scan_rx.next().await.expect("Scan should complete");
            }

            // Ensure no active scans remain
            assert_eq!(
                active_tasks.load(Ordering::SeqCst),
                0,
                "Scans not completed"
            );

            let queue = watcher_handle.read(&app, |watcher, _| watcher.processing_queue.clone());
            wait_for_queue_complete(queue, &mut app).await;
        });
    });
}

#[test]
fn test_scan_queue_handles_nonexistent_subscriber() {
    VirtualFS::test("scan_queue_nonexistent_subscriber", |dirs, mut vfs| {
        // Create a git repository
        stub_git_repository(&mut vfs, "repo");

        let repo_path = dirs.tests().join("repo");

        App::test((), |mut app| async move {
            let watcher_handle = app.add_singleton_model(DirectoryWatcher::new);

            // Create repository
            let repo_handle = watcher_handle
                .update(&mut app, |watcher, ctx| {
                    watcher.add_directory(
                        StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                        ctx,
                    )
                })
                .unwrap();

            // Manually enqueue a scan request for a non-existent subscriber
            watcher_handle.update(&mut app, |watcher, ctx| {
                watcher.processing_queue.update(ctx, |queue, ctx| {
                    queue.enqueue_scan(repo_handle.clone().downgrade(), 999, ctx);
                    // Non-existent subscriber ID
                });
            });

            // Add a real subscriber to test that the queue continues processing
            let (tx, mut rx) = mpsc::unbounded::<()>();
            let (update_tx, _) = mpsc::unbounded::<RepositoryUpdate>();
            let active_scans = Arc::new(AtomicUsize::new(0));
            let subscriber = TestSubscriber::new(tx, update_tx, active_scans.clone());

            std::mem::drop(repo_handle.update(&mut app, |repo, ctx| {
                repo.start_watching(Box::new(subscriber), ctx)
            }));

            // Wait for processing to complete
            rx.next().await.unwrap();

            // Verify the real subscriber's scan completed despite the invalid scan request
            assert_eq!(
                active_scans.load(Ordering::SeqCst),
                0,
                "Real subscriber scan should complete"
            );

            let queue = watcher_handle.read(&app, |watcher, _| watcher.processing_queue.clone());
            wait_for_queue_complete(queue, &mut app).await;
        });
    });
}

#[test]
#[ignore = "CODE-1071 - test is flaky and needs to be fixed"]
fn test_file_updates_delivered() {
    env_logger::init();

    VirtualFS::test("file_updates_delivered", |dirs, mut vfs| {
        stub_git_repository(&mut vfs, "test_repo");
        vfs.with_files(vec![
            Stub::FileWithContent("test_repo/file1.txt", "content1"),
            Stub::FileWithContent("test_repo/file2.txt", "content2"),
        ]);

        let repo_path = dirs.tests().join("test_repo");
        App::test((), |mut app| async move {
            let watcher_handle = app.add_singleton_model(DirectoryWatcher::new);

            let repo_handle = watcher_handle
                .update(&mut app, |watcher, ctx| {
                    watcher.add_directory(
                        StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                        ctx,
                    )
                })
                .unwrap();

            let task_count = Arc::new(AtomicUsize::new(0));
            let (scan_tx, mut scan_rx) = mpsc::unbounded::<()>();
            let (update_tx, mut update_rx) = mpsc::unbounded::<RepositoryUpdate>();

            let subscriber_count = 3;
            for _ in 0..subscriber_count {
                let subscriber =
                    TestSubscriber::new(scan_tx.clone(), update_tx.clone(), task_count.clone());
                let start = repo_handle.update(&mut app, |repo, ctx| {
                    repo.start_watching(Box::new(subscriber), ctx)
                });
                start
                    .registration_future
                    .await
                    .expect("Failed to add subscriber");
            }

            // Wait for all initial scans to complete.
            for _ in 0..subscriber_count {
                scan_rx.next().await.expect("Scan should complete");
            }

            let file1_path = repo_path.join("file1.txt");
            let file2_path = repo_path.join("file2.txt");
            fs::write(&file1_path, "new content").expect("Updating file 1 failed");
            fs::write(&file2_path, "new content").expect("Updating file 2 failed");

            // Ensure that all subscribers receive the updates.
            for _ in 0..subscriber_count {
                let update = update_rx.next().await.expect("Update should complete");
                let mut changed: Vec<_> =
                    update.into_added_or_modified().map(|tf| tf.path).collect();
                changed.sort();
                assert_eq!(changed, vec![file1_path.clone(), file2_path.clone()]);
            }

            let queue = watcher_handle.read(&app, |watcher, _| watcher.processing_queue.clone());
            wait_for_queue_complete(queue, &mut app).await;
        });
    });
}

/// Wait for a task queue to have finished all its tasks. This will not directly advance the queue, but allows the UI framework to mark async tasks as complete.
async fn wait_for_queue_complete(queue: ModelHandle<TaskQueue>, app: &mut App) {
    const DELAY: Duration = Duration::from_millis(5);
    const ATTEMPTS: u32 = 10;

    for _ in 0..ATTEMPTS {
        let complete = queue.update(app, |queue, _| {
            queue.active_tasks == 0 && queue.pending_tasks.is_empty()
        });
        if complete {
            return;
        }

        Timer::after(DELAY).await;
    }

    queue.read(app, |queue, _| {
        panic!(
            "Queue not empty after {:?}: {} pending tasks, {} active",
            DELAY * ATTEMPTS,
            queue.pending_tasks.len(),
            queue.active_tasks
        );
    })
}

#[test]
fn test_is_git_internal_path() {
    use crate::entry::is_git_internal_path;
    use std::path::Path;

    // .git/ internal paths should be detected
    assert!(is_git_internal_path(Path::new("/repo/.git/HEAD")));
    assert!(is_git_internal_path(Path::new("/repo/.git/index")));
    assert!(is_git_internal_path(Path::new(
        "/repo/.git/refs/heads/main"
    )));
    assert!(is_git_internal_path(Path::new("/repo/.git/config")));
    assert!(is_git_internal_path(Path::new("/repo/.git/objects/abc123")));
    assert!(is_git_internal_path(Path::new("/repo/.git")));

    #[cfg(windows)]
    assert!(is_git_internal_path(Path::new(r"C:\repo\.git\HEAD")));

    // Non-git files should not be detected
    assert!(!is_git_internal_path(Path::new("/repo/src/main.rs")));
    assert!(!is_git_internal_path(Path::new("/repo/README.md")));
}

#[test]
fn test_remote_tracking_ref_routes_only_to_repos_tracking_that_ref() {
    VirtualFS::test("remote_tracking_ref_routes", |dirs, mut vfs| {
        stub_git_repository(&mut vfs, "repo");
        vfs.mkdir("repo/.git/worktrees");
        vfs.mkdir("repo/.git/worktrees/wt");
        vfs.mkdir("repo/.git/refs/remotes");
        vfs.mkdir("repo/.git/refs/remotes/origin");
        vfs.mkdir("wt");
        vfs.with_files(vec![
            Stub::FileWithContent("repo/.git/refs/remotes/origin/main", "abc123"),
            Stub::FileWithContent("repo/.git/refs/remotes/origin/feature", "def456"),
            Stub::FileWithContent("repo/.git/worktrees/wt/HEAD", "ref: refs/heads/feature"),
        ]);

        let repo_path = dirs.tests().join("repo");
        let worktree_path = dirs.tests().join("wt");
        let external_git_dir = dirs.tests().join("repo/.git/worktrees/wt");
        let main_remote_ref_path = dirs.tests().join("repo/.git/refs/remotes/origin/main");
        let feature_remote_ref_path = dirs.tests().join("repo/.git/refs/remotes/origin/feature");

        App::test((), |mut app| async move {
            let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);

            let main_repo_handle = watcher_handle
                .update(&mut app, |watcher, ctx| {
                    watcher.add_directory(
                        StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                        ctx,
                    )
                })
                .unwrap();
            let worktree_repo_handle = watcher_handle
                .update(&mut app, |watcher, ctx| {
                    watcher.add_directory_with_git_dir(
                        StandardizedPath::from_local_canonicalized(&worktree_path).unwrap(),
                        Some(
                            StandardizedPath::from_local_canonicalized(&external_git_dir).unwrap(),
                        ),
                        ctx,
                    )
                })
                .unwrap();

            main_repo_handle.update(&mut app, |repo, _| {
                repo.update_tracked_remote_ref(TrackedRemoteRef::from_full_ref_name(
                    "refs/remotes/origin/main",
                ));
            });
            worktree_repo_handle.update(&mut app, |repo, _| {
                repo.update_tracked_remote_ref(TrackedRemoteRef::from_full_ref_name(
                    "refs/remotes/origin/feature",
                ));
            });

            let main_affected = watcher_handle.update(&mut app, |watcher, ctx| {
                watcher.find_repos_for_git_event(&main_remote_ref_path, ctx)
            });
            assert_eq!(main_affected, vec![main_repo_handle.clone()]);

            let feature_affected = watcher_handle.update(&mut app, |watcher, ctx| {
                watcher.find_repos_for_git_event(&feature_remote_ref_path, ctx)
            });
            assert_eq!(feature_affected, vec![worktree_repo_handle]);
        });
    });
}

#[test]
fn test_common_config_routes_to_repos_sharing_common_git_dir() {
    VirtualFS::test("common_config_routes", |dirs, mut vfs| {
        stub_git_repository(&mut vfs, "repo");
        vfs.mkdir("repo/.git/worktrees");
        vfs.mkdir("repo/.git/worktrees/wt");
        vfs.mkdir("wt");
        vfs.with_files(vec![Stub::FileWithContent(
            "repo/.git/worktrees/wt/HEAD",
            "ref: refs/heads/feature",
        )]);

        let repo_path = dirs.tests().join("repo");
        let worktree_path = dirs.tests().join("wt");
        let external_git_dir = dirs.tests().join("repo/.git/worktrees/wt");
        let common_config_path = dirs.tests().join("repo/.git/config");

        App::test((), |mut app| async move {
            let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);

            let main_repo_handle = watcher_handle
                .update(&mut app, |watcher, ctx| {
                    watcher.add_directory(
                        StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                        ctx,
                    )
                })
                .unwrap();
            let worktree_repo_handle = watcher_handle
                .update(&mut app, |watcher, ctx| {
                    watcher.add_directory_with_git_dir(
                        StandardizedPath::from_local_canonicalized(&worktree_path).unwrap(),
                        Some(
                            StandardizedPath::from_local_canonicalized(&external_git_dir).unwrap(),
                        ),
                        ctx,
                    )
                })
                .unwrap();

            let affected = watcher_handle.update(&mut app, |watcher, ctx| {
                watcher.find_repos_for_git_event(&common_config_path, ctx)
            });
            assert_eq!(affected.len(), 2);
            assert!(affected.contains(&main_repo_handle));
            assert!(affected.contains(&worktree_repo_handle));
        });
    });
}

#[test]
#[ignore = "flaky test: CODE-1492"]
fn test_commit_related_files_excluded_from_update_lists() {
    VirtualFS::test("commit_files_excluded", |dirs, mut vfs| {
        log::info!("Start setting up test vfs");
        stub_git_repository(&mut vfs, "test_repo");

        // Create .git/refs/heads directory structure
        vfs.mkdir("test_repo/.git/refs/heads");
        vfs.with_files(vec![
            Stub::FileWithContent("test_repo/.git/HEAD", "ref: refs/heads/main"),
            Stub::FileWithContent("test_repo/.git/refs/heads/main", "abc123def456"),
            Stub::FileWithContent("test_repo/regular_file.txt", "content"),
        ]);

        let repo_path = dirs.tests().join("test_repo");
        App::test((), |mut app| async move {
            let watcher_handle = app.add_singleton_model(DirectoryWatcher::new);

            let repo_handle = watcher_handle
                .update(&mut app, |watcher, ctx| {
                    watcher.add_directory(
                        StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                        ctx,
                    )
                })
                .unwrap();

            let (scan_tx, mut scan_rx) = mpsc::unbounded::<()>();
            let (update_tx, mut update_rx) = mpsc::unbounded::<RepositoryUpdate>();
            let active_scans = Arc::new(AtomicUsize::new(0));

            let subscriber =
                TestSubscriber::new(scan_tx.clone(), update_tx.clone(), active_scans.clone());

            log::info!("Start setting up watcher");
            let start = repo_handle.update(&mut app, |repo, ctx| {
                repo.start_watching(Box::new(subscriber), ctx)
            });
            start
                .registration_future
                .await
                .expect("Failed to add subscriber");
            log::info!("Finished setting up watcher");

            // Wait for initial scan to complete
            scan_rx.next().await.expect("Scan should complete");
            log::info!("Initial scan completed");

            // Update both a regular file and a commit-related file
            let regular_file_path = repo_path.join("regular_file.txt");
            let head_file_path = repo_path.join(".git/HEAD");
            let branch_file_path = repo_path.join(".git/refs/heads/main");

            std::fs::write(&regular_file_path, "new content")
                .expect("Updating regular file failed");
            std::fs::write(&head_file_path, "ref: refs/heads/feature")
                .expect("Updating HEAD failed");
            std::fs::write(&branch_file_path, "def456abc123").expect("Updating branch ref failed");
            log::info!("Wrote files: regular_file.txt, .git/HEAD, .git/refs/heads/main");

            // Receive the update with timeout and retry
            let update = loop {
                futures::select! {
                    update = futures::FutureExt::fuse(update_rx.next()) => {
                        match update {
                            Some(update) => {
                                log::info!("Received update");
                                break update;
                            }
                            None => {
                                panic!("Update channel closed unexpectedly");
                            }
                        }
                    }
                    _ = futures::FutureExt::fuse(Timer::after(Duration::from_secs(5))) => {
                        log::warn!("Waiting for update timed out after 5s, retrying...");
                    }
                }
            };

            // Verify that commit_updated is true
            assert!(
                update.commit_updated,
                "commit_updated should be true when git commit files change"
            );

            // Verify that git files are NOT in the added list, but regular files are
            use crate::TargetFile;
            assert!(
                update
                    .contains_added_or_modified(&TargetFile::new(regular_file_path.clone(), false)),
                "Regular file should be in added/modified list"
            );
            assert!(
                !update.contains_added_or_modified(&TargetFile::new(head_file_path.clone(), false)),
                "Git HEAD file should NOT be in added/modified list"
            );
            assert!(
                !update
                    .contains_added_or_modified(&TargetFile::new(branch_file_path.clone(), false)),
                "Git branch ref file should NOT be in added/modified list"
            );

            // The update should not be considered empty due to commit_updated being true
            assert!(
                !update.is_empty(),
                "Update should not be empty when commit_updated is true"
            );

            let queue = watcher_handle.read(&app, |watcher, _| watcher.processing_queue.clone());
            wait_for_queue_complete(queue, &mut app).await;
        });
    });
}
