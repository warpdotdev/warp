use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use futures::channel::mpsc;
use futures::{FutureExt as _, StreamExt as _};
use virtual_fs::{Stub, VirtualFS};
use warp_util::standardized_path::StandardizedPath;
use warpui_core::r#async::Timer;
use warpui_core::{App, ModelContext};

use super::{
    Repository, RepositorySubscriber, RepositorySubscription, RepositoryWatchMode,
    TrackedRemoteRef, merge_repository_updates,
};
use crate::repositories::stub_git_repository;
use crate::watcher::DirectoryWatcher;
use crate::{RepositoryUpdate, TargetFile};

struct RecordingSubscriber {
    update_tx: mpsc::UnboundedSender<RepositoryUpdate>,
}

impl RepositorySubscriber for RecordingSubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        Box::pin(async {})
    }

    fn on_files_updated(
        &mut self,
        _repository: &Repository,
        update: &RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let update = update.clone();
        let update_tx = self.update_tx.clone();
        Box::pin(async move {
            let _ = update_tx.unbounded_send(update);
        })
    }
}

fn add_recording_subscriber(
    repository: &mut Repository,
    mode: RepositoryWatchMode,
    update_tx: mpsc::UnboundedSender<RepositoryUpdate>,
) -> usize {
    let subscriber_id = repository.next_subscriber_id;
    repository.next_subscriber_id += 1;
    repository.subscribers.insert(
        subscriber_id,
        RepositorySubscription {
            mode,
            subscriber: Box::new(RecordingSubscriber { update_tx }),
        },
    );
    subscriber_id
}
#[test]
fn tracked_remote_ref_validates_full_ref_names() {
    assert_eq!(
        TrackedRemoteRef::from_full_ref_name("refs/remotes/origin/main")
            .unwrap()
            .full_ref_name(),
        "refs/remotes/origin/main"
    );
    assert_eq!(
        TrackedRemoteRef::from_full_ref_name("refs/remotes/origin/feature/nested")
            .unwrap()
            .full_ref_name(),
        "refs/remotes/origin/feature/nested"
    );

    assert!(TrackedRemoteRef::from_full_ref_name("refs/heads/main").is_none());
    assert!(TrackedRemoteRef::from_full_ref_name("refs/remotes/origin").is_none());
    assert!(TrackedRemoteRef::from_full_ref_name("/refs/remotes/origin/main").is_none());
    assert!(TrackedRemoteRef::from_full_ref_name("refs/remotes/origin/../main").is_none());
}

#[test]
fn tracked_remote_ref_path_uses_common_git_dir() {
    VirtualFS::test(
        "tracked_remote_ref_path_uses_common_git_dir",
        |dirs, mut vfs| {
            stub_git_repository(&mut vfs, "repo");
            vfs.mkdir("repo/.git/refs/remotes");
            vfs.mkdir("repo/.git/refs/remotes/origin");
            vfs.with_files(vec![Stub::FileWithContent(
                "repo/.git/refs/remotes/origin/main",
                "abc123",
            )]);

            let repo_path = dirs.tests().join("repo");
            let remote_ref_path = repo_path.join(".git/refs/remotes/origin/main");

            App::test((), |mut app| async move {
                let watcher_handle = app.add_model(DirectoryWatcher::new_for_testing);
                let repo_handle = watcher_handle
                    .update(&mut app, |watcher, ctx| {
                        watcher.add_directory(
                            StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                            ctx,
                        )
                    })
                    .unwrap();

                repo_handle.update(&mut app, |repo, _| {
                    assert!(
                        repo.update_tracked_remote_ref(TrackedRemoteRef::from_full_ref_name(
                            "refs/remotes/origin/main"
                        ))
                    );
                    assert_eq!(
                        repo.tracked_remote_ref_path(),
                        Some(remote_ref_path.clone())
                    );
                    assert!(repo.tracks_remote_ref_path(&remote_ref_path));
                });
            });
        },
    );
}

#[test]
fn tracked_remote_ref_path_uses_linked_worktree_common_git_dir() {
    VirtualFS::test(
        "tracked_remote_ref_path_uses_linked_worktree_common_git_dir",
        |dirs, mut vfs| {
            stub_git_repository(&mut vfs, "repo");
            vfs.mkdir("repo/.git/worktrees");
            vfs.mkdir("repo/.git/worktrees/wt");
            vfs.mkdir("repo/.git/refs/remotes");
            vfs.mkdir("repo/.git/refs/remotes/origin");
            vfs.mkdir("wt");
            vfs.with_files(vec![
                Stub::FileWithContent("repo/.git/worktrees/wt/HEAD", "ref: refs/heads/feature"),
                Stub::FileWithContent("repo/.git/refs/remotes/origin/feature", "abc123"),
            ]);

            let worktree_path = dirs.tests().join("wt");
            let external_git_dir = dirs.tests().join("repo/.git/worktrees/wt");
            let remote_ref_path = dirs.tests().join("repo/.git/refs/remotes/origin/feature");

            App::test((), |mut app| async move {
                let watcher_handle = app.add_model(DirectoryWatcher::new_for_testing);
                let repo_handle = watcher_handle
                    .update(&mut app, |watcher, ctx| {
                        watcher.add_directory_with_git_dir(
                            StandardizedPath::from_local_canonicalized(&worktree_path).unwrap(),
                            Some(
                                StandardizedPath::from_local_canonicalized(&external_git_dir)
                                    .unwrap(),
                            ),
                            ctx,
                        )
                    })
                    .unwrap();

                repo_handle.update(&mut app, |repo, _| {
                    assert!(
                        repo.update_tracked_remote_ref(TrackedRemoteRef::from_full_ref_name(
                            "refs/remotes/origin/feature"
                        ))
                    );
                    assert_eq!(
                        repo.tracked_remote_ref_path(),
                        Some(remote_ref_path.clone())
                    );
                    assert!(repo.tracks_remote_ref_path(&remote_ref_path));
                });
            });
        },
    );
}

#[test]
fn merge_repository_updates_preserves_remote_ref_updates() {
    let mut acc = RepositoryUpdate {
        added: [TargetFile::new(PathBuf::from("/repo/file.txt"), false)].into(),
        ..Default::default()
    };
    let incoming = RepositoryUpdate {
        remote_ref_updated: true,
        ..Default::default()
    };

    merge_repository_updates(&mut acc, &incoming);

    assert!(acc.remote_ref_updated);
    assert!(
        acc.added
            .contains(&TargetFile::new(PathBuf::from("/repo/file.txt"), false))
    );
}

#[test]
fn filesystem_only_subscription_does_not_activate_git_tracking() {
    VirtualFS::test(
        "filesystem_only_subscription_does_not_activate_git_tracking",
        |dirs, mut vfs| {
            stub_git_repository(&mut vfs, "repo");
            let repo_path = dirs.tests().join("repo");

            App::test((), |mut app| async move {
                let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
                let repo_handle = watcher_handle
                    .update(&mut app, |watcher, ctx| {
                        watcher.add_directory(
                            StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                            ctx,
                        )
                    })
                    .unwrap();

                let (update_tx, _) = mpsc::unbounded::<RepositoryUpdate>();
                let start = repo_handle.update(&mut app, |repo, ctx| {
                    repo.start_watching(
                        RepositoryWatchMode::FilesystemOnly,
                        Box::new(RecordingSubscriber { update_tx }),
                        ctx,
                    )
                });
                std::mem::drop(start.registration_future);

                repo_handle.read(&app, |repo, _| {
                    assert!(!repo.has_git_repository_subscribers());
                    assert_eq!(repo.tracked_remote_ref_refresh_count, 0);
                });
            });
        },
    );
}

#[test]
fn git_tracking_activates_once_for_concurrent_git_subscriptions() {
    VirtualFS::test(
        "git_tracking_activates_once_for_concurrent_git_subscriptions",
        |dirs, mut vfs| {
            stub_git_repository(&mut vfs, "repo");
            let repo_path = dirs.tests().join("repo");

            App::test((), |mut app| async move {
                let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
                let repo_handle = watcher_handle
                    .update(&mut app, |watcher, ctx| {
                        watcher.add_directory(
                            StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                            ctx,
                        )
                    })
                    .unwrap();

                let mut subscriber_ids = Vec::new();
                for _ in 0..2 {
                    let (update_tx, _) = mpsc::unbounded::<RepositoryUpdate>();
                    let start = repo_handle.update(&mut app, |repo, ctx| {
                        repo.start_watching(
                            RepositoryWatchMode::GitRepository,
                            Box::new(RecordingSubscriber { update_tx }),
                            ctx,
                        )
                    });
                    subscriber_ids.push(start.subscriber_id);
                    std::mem::drop(start.registration_future);
                }

                repo_handle.read(&app, |repo, _| {
                    assert!(repo.has_git_repository_subscribers());
                    assert_eq!(repo.tracked_remote_ref_refresh_count, 1);
                });

                for subscriber_id in subscriber_ids {
                    repo_handle.update(&mut app, |repo, ctx| {
                        repo.stop_watching(subscriber_id, ctx);
                    });
                }

                repo_handle.read(&app, |repo, _| {
                    assert!(!repo.has_git_repository_subscribers());
                    assert!(repo.tracked_remote_ref.is_none());
                });
            });
        },
    );
}

#[test]
fn mixed_watch_modes_filter_git_flags_per_subscriber() {
    VirtualFS::test(
        "mixed_watch_modes_filter_git_flags_per_subscriber",
        |dirs, mut vfs| {
            stub_git_repository(&mut vfs, "repo");
            let repo_path = dirs.tests().join("repo");

            App::test((), |mut app| async move {
                let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
                let repo_handle = watcher_handle
                    .update(&mut app, |watcher, ctx| {
                        watcher.add_directory(
                            StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                            ctx,
                        )
                    })
                    .unwrap();

                let (filesystem_tx, _) = mpsc::unbounded::<RepositoryUpdate>();
                let (git_tx, _) = mpsc::unbounded::<RepositoryUpdate>();
                let (filesystem_id, git_id) = repo_handle.update(&mut app, |repo, _| {
                    (
                        add_recording_subscriber(
                            repo,
                            RepositoryWatchMode::FilesystemOnly,
                            filesystem_tx,
                        ),
                        add_recording_subscriber(repo, RepositoryWatchMode::GitRepository, git_tx),
                    )
                });
                let changed_file = TargetFile::new(repo_path.join("file.txt"), false);
                let update = RepositoryUpdate {
                    modified: [changed_file.clone()].into(),
                    commit_updated: true,
                    index_lock_detected: true,
                    remote_ref_updated: true,
                    ..Default::default()
                };

                let subscriber_updates =
                    repo_handle.read(&app, |repo, _| repo.subscriber_updates(&update));
                let filesystem_update = subscriber_updates
                    .iter()
                    .find_map(|(id, update)| (*id == filesystem_id).then_some(update))
                    .unwrap();
                let git_update = subscriber_updates
                    .iter()
                    .find_map(|(id, update)| (*id == git_id).then_some(update))
                    .unwrap();

                assert_eq!(filesystem_update.modified, [changed_file.clone()].into());
                assert!(!filesystem_update.commit_updated);
                assert!(!filesystem_update.index_lock_detected);
                assert!(!filesystem_update.remote_ref_updated);
                assert_eq!(git_update.modified, [changed_file].into());
                assert!(git_update.commit_updated);
                assert!(git_update.index_lock_detected);
                assert!(git_update.remote_ref_updated);
            });
        },
    );
}

#[test]
fn filesystem_only_subscription_drops_git_only_updates() {
    VirtualFS::test(
        "filesystem_only_subscription_drops_git_only_updates",
        |dirs, mut vfs| {
            stub_git_repository(&mut vfs, "repo");
            let repo_path = dirs.tests().join("repo");

            App::test((), |mut app| async move {
                let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
                let repo_handle = watcher_handle
                    .update(&mut app, |watcher, ctx| {
                        watcher.add_directory(
                            StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                            ctx,
                        )
                    })
                    .unwrap();

                let (update_tx, _) = mpsc::unbounded::<RepositoryUpdate>();
                repo_handle.update(&mut app, |repo, _| {
                    add_recording_subscriber(repo, RepositoryWatchMode::FilesystemOnly, update_tx);
                });
                let update = RepositoryUpdate {
                    commit_updated: true,
                    ..Default::default()
                };

                let subscriber_updates =
                    repo_handle.read(&app, |repo, _| repo.subscriber_updates(&update));

                assert!(subscriber_updates.is_empty());
            });
        },
    );
}

#[test]
fn tracked_remote_ref_change_notifies_subscribers() {
    VirtualFS::test("tracked_remote_ref_change_notifies", |dirs, mut vfs| {
        stub_git_repository(&mut vfs, "repo");

        let repo_path = dirs.tests().join("repo");

        App::test((), |mut app| async move {
            let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
            let repo_handle = watcher_handle
                .update(&mut app, |watcher, ctx| {
                    watcher.add_directory(
                        StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                        ctx,
                    )
                })
                .unwrap();

            let (update_tx, mut update_rx) = mpsc::unbounded::<RepositoryUpdate>();
            repo_handle.update(&mut app, |repo, _| {
                add_recording_subscriber(repo, RepositoryWatchMode::GitRepository, update_tx);
            });

            repo_handle.update(&mut app, |repo, ctx| {
                if repo.update_tracked_remote_ref(TrackedRemoteRef::from_full_ref_name(
                    "refs/remotes/origin/main",
                )) {
                    repo.enqueue_remote_ref_update(ctx);
                }
            });

            let update = update_rx.next().await.expect("remote ref update");
            assert!(update.remote_ref_updated);
            assert!(!update.commit_updated);
            assert!(!update.index_lock_detected);
            assert!(update.added.is_empty());
            assert!(update.modified.is_empty());
            assert!(update.deleted.is_empty());
            assert!(update.moved.is_empty());
        });
    });
}

#[test]
fn unchanged_tracked_remote_ref_does_not_notify_subscribers() {
    VirtualFS::test(
        "unchanged_tracked_remote_ref_does_not_notify",
        |dirs, mut vfs| {
            stub_git_repository(&mut vfs, "repo");

            let repo_path = dirs.tests().join("repo");

            App::test((), |mut app| async move {
                let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
                let repo_handle = watcher_handle
                    .update(&mut app, |watcher, ctx| {
                        watcher.add_directory(
                            StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                            ctx,
                        )
                    })
                    .unwrap();

                let (update_tx, mut update_rx) = mpsc::unbounded::<RepositoryUpdate>();
                repo_handle.update(&mut app, |repo, _| {
                    add_recording_subscriber(repo, RepositoryWatchMode::GitRepository, update_tx);
                });

                repo_handle.update(&mut app, |repo, _| {
                    repo.update_tracked_remote_ref(TrackedRemoteRef::from_full_ref_name(
                        "refs/remotes/origin/main",
                    ));
                });
                repo_handle.update(&mut app, |repo, ctx| {
                    if repo.update_tracked_remote_ref(TrackedRemoteRef::from_full_ref_name(
                        "refs/remotes/origin/main",
                    )) {
                        repo.enqueue_remote_ref_update(ctx);
                    }
                });

                futures::select! {
                    update = update_rx.next().fuse() => {
                        panic!("unexpected remote ref update: {update:?}");
                    }
                _ = futures::FutureExt::fuse(Timer::after(Duration::from_millis(100))) => {}
                }
            });
        },
    );
}
