use super::{merge_repository_updates, Repository, RepositorySubscriber, TrackedRemoteRef};
use crate::repositories::stub_git_repository;
use crate::watcher::DirectoryWatcher;
use crate::{RepositoryUpdate, TargetFile};
use futures::channel::mpsc;
use futures::{FutureExt as _, StreamExt as _};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;
use virtual_fs::{Stub, VirtualFS};
use warp_util::standardized_path::StandardizedPath;
use warpui::r#async::Timer;
use warpui::App;
use warpui::ModelContext;

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
    update_tx: mpsc::UnboundedSender<RepositoryUpdate>,
) {
    let subscriber_id = repository.next_subscriber_id;
    repository.next_subscriber_id += 1;
    repository
        .subscribers
        .insert(subscriber_id, Box::new(RecordingSubscriber { update_tx }));
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
    assert!(acc
        .added
        .contains(&TargetFile::new(PathBuf::from("/repo/file.txt"), false)));
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
                add_recording_subscriber(repo, update_tx);
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
                    add_recording_subscriber(repo, update_tx);
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
