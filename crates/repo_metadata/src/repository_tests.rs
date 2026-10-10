use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::channel::mpsc;
use futures::{FutureExt as _, StreamExt as _};
use virtual_fs::{Stub, VirtualFS};
use warp_util::standardized_path::StandardizedPath;
use warpui_core::r#async::Timer;
use warpui_core::{App, ModelContext};

use super::{
    BufferingRepositorySubscriber, Repository, RepositorySubscriber, RepositorySubscription,
    RepositoryWatchMode, TrackedRemoteRef, merge_repository_updates,
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

/// A subscriber whose `on_files_updated` takes `delay` to resolve, and asserts that it is never
/// invoked again before the previous call's returned future has resolved. Used to prove that
/// `BufferingRepositorySubscriber` serializes delivery instead of racing overlapping calls.
struct SlowRecordingSubscriber {
    update_tx: mpsc::UnboundedSender<RepositoryUpdate>,
    in_flight: Arc<AtomicBool>,
    delay: Duration,
}

impl RepositorySubscriber for SlowRecordingSubscriber {
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
        assert!(
            !self.in_flight.swap(true, Ordering::SeqCst),
            "on_files_updated was invoked again before the previous call's future resolved"
        );
        let update = update.clone();
        let update_tx = self.update_tx.clone();
        let in_flight = Arc::clone(&self.in_flight);
        let delay = self.delay;
        Box::pin(async move {
            Timer::after(delay).await;
            let _ = update_tx.unbounded_send(update);
            in_flight.store(false, Ordering::SeqCst);
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

/// Awaits `update_rx` until every file in `all_files` has been seen across delivered batches
/// (failing on any duplicate along the way), or `deadline` elapses first -- in which case it
/// panics with the batches seen so far and the files still missing, instead of hanging until
/// nextest's slow-timeout. `on_batch` is called with the 0-based batch index and the batch
/// itself before its files are recorded, so callers can assert per-batch properties without
/// reimplementing the receive loop. Returns the received files and the number of batches.
async fn collect_until_seen(
    update_rx: &mut mpsc::UnboundedReceiver<RepositoryUpdate>,
    all_files: &[TargetFile],
    deadline: Duration,
    mut on_batch: impl FnMut(usize, &RepositoryUpdate),
) -> (HashSet<TargetFile>, usize) {
    let mut seen = HashSet::new();
    let mut batch_count = 0;
    let sleep = futures::FutureExt::fuse(Timer::after(deadline));
    futures::pin_mut!(sleep);
    while seen.len() < all_files.len() {
        futures::select! {
            flushed = update_rx.next().fuse() => {
                let flushed = flushed.expect("channel closed before every update was seen");
                on_batch(batch_count, &flushed);
                for file in flushed.added {
                    assert!(seen.insert(file), "duplicate file delivered across batches");
                }
                batch_count += 1;
            }
            _ = sleep => {
                let missing: Vec<_> = all_files.iter().filter(|f| !seen.contains(f)).collect();
                panic!(
                    "timed out after {batch_count} batch(es) waiting for the rest to be \
                     delivered; missing: {missing:?}"
                );
            }
        }
    }
    (seen, batch_count)
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
fn merge_repository_updates_applies_every_entry_when_a_move_collapses_into_added() {
    let source = TargetFile::new(PathBuf::from("/repo/source.txt"), false);
    let move_target = TargetFile::new(PathBuf::from("/repo/moved.txt"), false);
    let unrelated_move_to = TargetFile::new(PathBuf::from("/repo/d.txt"), false);
    let unrelated_move_from = TargetFile::new(PathBuf::from("/repo/c.txt"), false);
    let added_file = TargetFile::new(PathBuf::from("/repo/added.txt"), false);
    let deleted_file = TargetFile::new(PathBuf::from("/repo/deleted.txt"), false);

    // `acc` already records `source` as added, so the incoming batch's move collapses into it
    // via the `acc.added.remove(from)` branch.
    let mut acc = RepositoryUpdate {
        added: [source.clone()].into(),
        ..Default::default()
    };

    let incoming = RepositoryUpdate {
        // A pre-existing bug used `return` instead of `continue` in this branch, which would
        // abandon the unrelated move plus every later phase below.
        moved: [(move_target.clone(), source.clone())].into(),
        added: [added_file.clone()].into(),
        deleted: [deleted_file.clone()].into(),
        remote_ref_updated: true,
        ..Default::default()
    };
    // A second, independent move exercises that the moves loop itself keeps iterating (not
    // just the phases after it); kept separate to avoid HashMap-iteration-order ambiguity with
    // the colliding move above.
    let mut incoming_with_second_move = incoming.clone();
    incoming_with_second_move
        .moved
        .insert(unrelated_move_to.clone(), unrelated_move_from.clone());

    merge_repository_updates(&mut acc, &incoming_with_second_move);

    assert!(
        acc.added.contains(&move_target),
        "the colliding move should collapse into `added`"
    );
    assert!(!acc.added.contains(&source));
    assert_eq!(
        acc.moved.get(&unrelated_move_to),
        Some(&unrelated_move_from),
        "the unrelated move must still be recorded"
    );
    assert!(
        acc.added.contains(&added_file),
        "adds after the moves phase must still be applied"
    );
    assert!(
        acc.deleted.contains(&deleted_file),
        "deletes after the moves phase must still be applied"
    );
    assert!(acc.remote_ref_updated, "flags must still be folded in");
}

#[test]
fn merge_repository_updates_applies_every_entry_when_a_move_collapses_into_modified() {
    let source = TargetFile::new(PathBuf::from("/repo/source.txt"), false);
    let move_target = TargetFile::new(PathBuf::from("/repo/moved.txt"), false);
    let unrelated_move_to = TargetFile::new(PathBuf::from("/repo/d.txt"), false);
    let unrelated_move_from = TargetFile::new(PathBuf::from("/repo/c.txt"), false);
    let added_file = TargetFile::new(PathBuf::from("/repo/added.txt"), false);

    // `acc` already records `source` as modified, so the incoming batch's move collapses into
    // it via the `acc.modified.remove(from)` branch.
    let mut acc = RepositoryUpdate {
        modified: [source.clone()].into(),
        ..Default::default()
    };

    let mut incoming = RepositoryUpdate {
        moved: [(move_target.clone(), source.clone())].into(),
        added: [added_file.clone()].into(),
        ..Default::default()
    };
    incoming
        .moved
        .insert(unrelated_move_to.clone(), unrelated_move_from.clone());

    merge_repository_updates(&mut acc, &incoming);

    assert!(
        acc.modified.contains(&move_target),
        "the colliding move should collapse into `modified`"
    );
    assert!(!acc.modified.contains(&source));
    assert_eq!(
        acc.moved.get(&unrelated_move_to),
        Some(&unrelated_move_from),
        "the unrelated move must still be recorded"
    );
    assert!(
        acc.added.contains(&added_file),
        "adds after the moves phase must still be applied"
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
fn forced_flush_drains_pending_before_debounce_timer_fires() {
    VirtualFS::test(
        "forced_flush_drains_pending_before_debounce_timer_fires",
        |dirs, mut vfs| {
            vfs.mkdir("repo");
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

                const MAX_PENDING_ENTRIES: usize = 3;
                let (update_tx, mut update_rx) = mpsc::unbounded::<RepositoryUpdate>();
                let start = repo_handle.update(&mut app, |repo, ctx| {
                    let buffered = BufferingRepositorySubscriber::with_max_pending_entries(
                        RecordingSubscriber { update_tx },
                        // Long enough that the debounce timer cannot plausibly fire during
                        // the test, so any observed flush must come from the forced path.
                        Duration::from_secs(3600),
                        MAX_PENDING_ENTRIES,
                    );
                    repo.start_watching(
                        RepositoryWatchMode::FilesystemOnly,
                        Box::new(buffered),
                        ctx,
                    )
                });
                std::mem::drop(start.registration_future);
                let subscriber_id = start.subscriber_id;

                let expected_files: Vec<_> = (0..MAX_PENDING_ENTRIES)
                    .map(|i| TargetFile::new(repo_path.join(format!("file{i}.txt")), false))
                    .collect();
                for file in &expected_files {
                    let update = RepositoryUpdate {
                        added: [file.clone()].into(),
                        ..Default::default()
                    };
                    repo_handle.update(&mut app, |repo, ctx| {
                        repo.notify_subscriber(subscriber_id, &update, ctx);
                    });
                }

                let flushed = update_rx.next().await.expect("forced flush");
                assert_eq!(flushed.added.len(), MAX_PENDING_ENTRIES);
                for file in &expected_files {
                    assert!(flushed.added.contains(file));
                }
            });
        },
    );
}

#[test]
fn single_incoming_update_exceeding_the_bound_is_delivered_without_loss() {
    VirtualFS::test(
        "single_incoming_update_exceeding_the_bound_is_delivered_without_loss",
        |dirs, mut vfs| {
            vfs.mkdir("repo");
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

                const MAX_PENDING_ENTRIES: usize = 4;
                let (update_tx, mut update_rx) = mpsc::unbounded::<RepositoryUpdate>();
                let start = repo_handle.update(&mut app, |repo, ctx| {
                    let buffered = BufferingRepositorySubscriber::with_max_pending_entries(
                        RecordingSubscriber { update_tx },
                        // Long enough that the debounce timer cannot plausibly fire during
                        // the test, so every observed batch must come from the forced,
                        // within-a-single-update chunking path.
                        Duration::from_secs(3600),
                        MAX_PENDING_ENTRIES,
                    );
                    repo.start_watching(
                        RepositoryWatchMode::FilesystemOnly,
                        Box::new(buffered),
                        ctx,
                    )
                });
                std::mem::drop(start.registration_future);
                let subscriber_id = start.subscriber_id;

                // A single incoming update far larger than the configured bound, exactly the
                // large-single-event case a coalesced filesystem-watcher batch can produce (a
                // huge git checkout or npm install landing as one `RepositoryUpdate`). A clean
                // multiple of the bound so nothing is left waiting on the (unreached) debounce.
                let total_files = MAX_PENDING_ENTRIES * 3;
                let all_files: Vec<_> = (0..total_files)
                    .map(|i| TargetFile::new(repo_path.join(format!("file{i}.txt")), false))
                    .collect();
                let huge_update = RepositoryUpdate {
                    added: all_files.iter().cloned().collect(),
                    ..Default::default()
                };

                repo_handle.update(&mut app, |repo, ctx| {
                    repo.notify_subscriber(subscriber_id, &huge_update, ctx);
                });

                let (seen, batch_count) = collect_until_seen(
                    &mut update_rx,
                    &all_files,
                    Duration::from_secs(5),
                    |index, flushed| {
                        if index == 0 {
                            // Nothing else is in flight yet the first time the bound is
                            // crossed, so that first batch is delivered immediately, on its
                            // own, bounded.
                            assert_eq!(flushed.added.len(), MAX_PENDING_ENTRIES);
                        }
                    },
                )
                .await;

                assert_eq!(seen.len(), total_files);
                for file in &all_files {
                    assert!(seen.contains(file));
                }
                // Everything after the first bounded batch coalesces into a single backlog
                // while that first delivery is in flight (see `BufferState::next_delivery`), so
                // this is only guaranteed to be more than one batch, not every batch bounded.
                assert!(
                    batch_count > 1,
                    "a single update far exceeding the bound should still be split into more than one batch"
                );
            });
        },
    );
}

#[test]
fn forced_and_debounced_flushes_apply_every_update_exactly_once_with_a_slow_consumer() {
    VirtualFS::test(
        "forced_and_debounced_flushes_apply_every_update_exactly_once_with_a_slow_consumer",
        |dirs, mut vfs| {
            vfs.mkdir("repo");
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

                const MAX_PENDING_ENTRIES: usize = 5;
                // Short enough to elapse comfortably within the test's timeout, but long enough
                // that the feed loop below (which never awaits between updates) reliably
                // finishes well before it fires.
                const DEBOUNCE: Duration = Duration::from_millis(30);
                // Longer than `DEBOUNCE`, so the debounce timer can fire while a forced batch is
                // still being delivered -- exercising the case where a debounce-triggered flush
                // has to wait behind an in-flight forced delivery instead of racing it.
                const CONSUMER_DELAY: Duration = Duration::from_millis(40);

                let (update_tx, mut update_rx) = mpsc::unbounded::<RepositoryUpdate>();
                let in_flight = Arc::new(AtomicBool::new(false));
                let start = repo_handle.update(&mut app, |repo, ctx| {
                    let buffered = BufferingRepositorySubscriber::with_max_pending_entries(
                        SlowRecordingSubscriber {
                            update_tx,
                            in_flight: Arc::clone(&in_flight),
                            delay: CONSUMER_DELAY,
                        },
                        DEBOUNCE,
                        MAX_PENDING_ENTRIES,
                    );
                    repo.start_watching(
                        RepositoryWatchMode::FilesystemOnly,
                        Box::new(buffered),
                        ctx,
                    )
                });
                std::mem::drop(start.registration_future);
                let subscriber_id = start.subscriber_id;

                // Two forced flushes' worth, plus a partial batch that only the debounce timer
                // flushes once the feed goes quiet.
                let total_updates = MAX_PENDING_ENTRIES * 2 + 3;
                let all_files: Vec<_> = (0..total_updates)
                    .map(|i| TargetFile::new(repo_path.join(format!("file{i}.txt")), false))
                    .collect();
                for file in &all_files {
                    let update = RepositoryUpdate {
                        added: [file.clone()].into(),
                        ..Default::default()
                    };
                    repo_handle.update(&mut app, |repo, ctx| {
                        repo.notify_subscriber(subscriber_id, &update, ctx);
                    });
                }

                // Some number of forced and/or debounced batches account for every update --
                // exactly how many depends on timing (a forced batch that's still in flight
                // when more work becomes ready coalesces that work into one backlog rather than
                // a separate batch), but nothing may ever be dropped or duplicated.
                // `SlowRecordingSubscriber` itself asserts that no two batches are ever
                // delivered concurrently.
                let (seen, _batch_count) = collect_until_seen(
                    &mut update_rx,
                    &all_files,
                    Duration::from_secs(5),
                    |_, flushed| assert!(!flushed.added.is_empty()),
                )
                .await;

                assert_eq!(seen.len(), total_updates);
                for file in &all_files {
                    assert!(seen.contains(file));
                }

                // Nothing else should ever arrive.
                futures::select! {
                    update = update_rx.next().fuse() => {
                        panic!("unexpected extra flush: {update:?}");
                    }
                    _ = futures::FutureExt::fuse(Timer::after(Duration::from_millis(300))) => {}
                }
            });
        },
    );
}

#[test]
fn unsubscribe_cancels_pending_delivery_and_releases_the_backlog() {
    VirtualFS::test(
        "unsubscribe_cancels_pending_delivery_and_releases_the_backlog",
        |dirs, mut vfs| {
            vfs.mkdir("repo");
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

                const MAX_PENDING_ENTRIES: usize = 3;
                let (update_tx, mut update_rx) = mpsc::unbounded::<RepositoryUpdate>();
                let in_flight = Arc::new(AtomicBool::new(false));
                let start = repo_handle.update(&mut app, |repo, ctx| {
                    let buffered = BufferingRepositorySubscriber::with_max_pending_entries(
                        SlowRecordingSubscriber {
                            update_tx,
                            in_flight: Arc::clone(&in_flight),
                            delay: Duration::from_millis(50),
                        },
                        Duration::from_secs(3600),
                        MAX_PENDING_ENTRIES,
                    );
                    repo.start_watching(
                        RepositoryWatchMode::FilesystemOnly,
                        Box::new(buffered),
                        ctx,
                    )
                });
                std::mem::drop(start.registration_future);
                let subscriber_id = start.subscriber_id;

                // First threshold crossing: dispatched immediately, taking 50ms to resolve.
                // Second threshold crossing: nothing is in flight, so it coalesces into the
                // pending backlog instead of being delivered.
                let total_updates = MAX_PENDING_ENTRIES * 2;
                for i in 0..total_updates {
                    let update = RepositoryUpdate {
                        added: [TargetFile::new(
                            repo_path.join(format!("file{i}.txt")),
                            false,
                        )]
                        .into(),
                        ..Default::default()
                    };
                    repo_handle.update(&mut app, |repo, ctx| {
                        repo.notify_subscriber(subscriber_id, &update, ctx);
                    });
                }

                // Unsubscribe while the first batch is still in flight and the second is only
                // backlogged.
                repo_handle.update(&mut app, |repo, ctx| {
                    repo.stop_watching(subscriber_id, ctx);
                });

                // The already-in-flight first batch still completes normally...
                let flushed = update_rx
                    .next()
                    .await
                    .expect("the in-flight delivery still completes");
                assert_eq!(flushed.added.len(), MAX_PENDING_ENTRIES);

                // ...but the backlogged second batch must never be delivered: unsubscribing
                // released it instead of delivering it late. The channel closing (because
                // dropping the subscription released the last reference to the subscriber) is
                // an acceptable way for that to manifest, same as the timer simply elapsing.
                futures::select! {
                    result = update_rx.next().fuse() => {
                        if let Some(update) = result {
                            panic!("unexpected delivery after unsubscribe: {update:?}");
                        }
                    }
                    _ = futures::FutureExt::fuse(Timer::after(Duration::from_millis(200))) => {}
                }
            });
        },
    );
}

#[test]
fn coalescing_a_move_that_collapses_into_an_existing_add_still_applies_later_entries() {
    VirtualFS::test(
        "coalescing_a_move_that_collapses_into_an_existing_add_still_applies_later_entries",
        |dirs, mut vfs| {
            vfs.mkdir("repo");
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

                const MAX_PENDING_ENTRIES: usize = 3;
                let (update_tx, mut update_rx) = mpsc::unbounded::<RepositoryUpdate>();
                let in_flight = Arc::new(AtomicBool::new(false));
                let start = repo_handle.update(&mut app, |repo, ctx| {
                    let buffered = BufferingRepositorySubscriber::with_max_pending_entries(
                        SlowRecordingSubscriber {
                            update_tx,
                            in_flight: Arc::clone(&in_flight),
                            delay: Duration::from_millis(80),
                        },
                        Duration::from_secs(3600),
                        MAX_PENDING_ENTRIES,
                    );
                    repo.start_watching(
                        RepositoryWatchMode::FilesystemOnly,
                        Box::new(buffered),
                        ctx,
                    )
                });
                std::mem::drop(start.registration_future);
                let subscriber_id = start.subscriber_id;

                // Batch A: crosses the bound on its own and is dispatched immediately (nothing
                // else is in flight yet), taking 80ms to resolve.
                let a1 = TargetFile::new(repo_path.join("a1.txt"), false);
                let a2 = TargetFile::new(repo_path.join("a2.txt"), false);
                let a3 = TargetFile::new(repo_path.join("a3.txt"), false);
                let batch_a_update = RepositoryUpdate {
                    added: [a1.clone(), a2.clone(), a3.clone()].into(),
                    ..Default::default()
                };
                repo_handle.update(&mut app, |repo, ctx| {
                    repo.notify_subscriber(subscriber_id, &batch_a_update, ctx);
                });

                // Batch B: crosses the bound while A is still in flight, so it becomes the
                // initial `next_delivery` backlog via a plain assignment (nothing to merge with
                // yet).
                let move_target = TargetFile::new(repo_path.join("moved.txt"), false);
                let w1 = TargetFile::new(repo_path.join("w1.txt"), false);
                let w2 = TargetFile::new(repo_path.join("w2.txt"), false);
                let batch_b_update = RepositoryUpdate {
                    added: [move_target.clone(), w1.clone(), w2.clone()].into(),
                    ..Default::default()
                };
                repo_handle.update(&mut app, |repo, ctx| {
                    repo.notify_subscriber(subscriber_id, &batch_b_update, ctx);
                });

                // Batch C: also crosses the bound while A is still in flight, so `hand_off`
                // merges it into the existing `next_delivery` (batch B) via
                // `merge_repository_updates`. Its one move's source is `move_target`, which
                // batch B recorded as `added` -- the exact "a move collapses into an existing
                // add" condition a pre-existing bug (`return` instead of `continue` in the
                // moves loop) would abandon everything after, dropping `extra_added` and
                // `extra_deleted` below. A single move (not two) keeps this deterministic,
                // independent of `HashMap` iteration order.
                let moved_to = TargetFile::new(repo_path.join("moved_to.txt"), false);
                let extra_added = TargetFile::new(repo_path.join("extra_added.txt"), false);
                let extra_deleted = TargetFile::new(repo_path.join("extra_deleted.txt"), false);
                let batch_c_update = RepositoryUpdate {
                    moved: [(moved_to.clone(), move_target.clone())].into(),
                    added: [extra_added.clone()].into(),
                    deleted: [extra_deleted.clone()].into(),
                    ..Default::default()
                };
                repo_handle.update(&mut app, |repo, ctx| {
                    repo.notify_subscriber(subscriber_id, &batch_c_update, ctx);
                });

                // Batch A completes first.
                let flushed_a = update_rx.next().await.expect("batch A");
                assert_eq!(flushed_a.added, [a1, a2, a3].into());

                // The coalesced backlog (B merged with C) is delivered next, once A's in-flight
                // delivery completes.
                let flushed_backlog = update_rx.next().await.expect("coalesced backlog");
                assert!(
                    flushed_backlog.added.contains(&moved_to),
                    "the move's target should replace the source in `added`"
                );
                assert!(
                    !flushed_backlog.added.contains(&move_target),
                    "the move's source should no longer be recorded as `added`"
                );
                assert!(flushed_backlog.added.contains(&w1));
                assert!(flushed_backlog.added.contains(&w2));
                assert!(
                    flushed_backlog.added.contains(&extra_added),
                    "adds after the colliding move must still be applied"
                );
                assert!(
                    flushed_backlog.deleted.contains(&extra_deleted),
                    "deletes after the colliding move must still be applied"
                );
            });
        },
    );
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
