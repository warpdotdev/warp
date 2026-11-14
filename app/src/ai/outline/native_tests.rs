use std::time::Duration;

use ai::index::build_outline;
use repo_metadata::{DirectoryWatcher, RepositoryUpdate, TargetFile};
use tempfile::TempDir;
use warp_util::standardized_path::StandardizedPath;
use warpui::App;
use warpui_core::r#async::Timer;

use super::{OutlineState, OutlineStatus, RepoOutlines};

/// `OutlineRepositorySubscriber` resolves as soon as it enqueues onto its own channel, while the
/// real recomputation (`RepoOutlines::handle_repository_update`) keeps running for much longer.
/// This exercises that two-stage path directly: it drives an actual `Outline` recomputation via
/// `handle_repository_update` and submits a second update while the first is still in flight, to
/// prove the second is coalesced into the accumulator -- not dropped, and not starting a second,
/// overlapping recomputation.
#[test]
fn concurrent_update_while_pending_is_merged_not_dropped_or_double_recomputed() {
    App::test((), |mut app| async move {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = dunce::canonicalize(temp_dir.path()).unwrap();
        std::fs::write(repo_path.join("existing.rs"), "fn existing() {}\n").unwrap();

        let baseline_outline = build_outline(&repo_path, None).await.unwrap();
        assert_eq!(baseline_outline.file_count(), 1);

        let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
        let repository_handle = watcher_handle
            .update(&mut app, |watcher, ctx| {
                watcher.add_directory(
                    StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                    ctx,
                )
            })
            .unwrap();

        let outlines_handle = app.add_singleton_model(RepoOutlines::new_for_test);
        outlines_handle.update(&mut app, |outlines, _| {
            outlines.outlines.insert(
                repo_path.clone(),
                OutlineState {
                    repository: repository_handle,
                    status: OutlineStatus::Complete(baseline_outline),
                    subscriber_id: None,
                    pending_update: RepositoryUpdate::default(),
                },
            );
        });

        std::fs::write(repo_path.join("first.rs"), "fn first() {}\n").unwrap();
        std::fs::write(repo_path.join("second.rs"), "fn second() {}\n").unwrap();

        let first_update = RepositoryUpdate {
            added: [TargetFile::new(repo_path.join("first.rs"), false)].into(),
            ..Default::default()
        };
        let second_update = RepositoryUpdate {
            added: [TargetFile::new(repo_path.join("second.rs"), false)].into(),
            ..Default::default()
        };

        outlines_handle.update(&mut app, |outlines, ctx| {
            outlines.handle_repository_update(&repo_path, first_update, ctx);
        });

        // `handle_repository_update` flips `status` to `Pending` synchronously, before the
        // spawned recomputation is ever polled, so this deterministically observes it in flight.
        outlines_handle.read(&app, |outlines, _| {
            let state = outlines.outlines.get(&repo_path).unwrap();
            assert!(matches!(state.status, OutlineStatus::Pending));
        });

        outlines_handle.update(&mut app, |outlines, ctx| {
            outlines.handle_repository_update(&repo_path, second_update.clone(), ctx);
        });

        // It must be merged into the accumulator: still `Pending` (no second, overlapping
        // recomputation was started), and the accumulator holds exactly the second update.
        outlines_handle.read(&app, |outlines, _| {
            let state = outlines.outlines.get(&repo_path).unwrap();
            assert!(matches!(state.status, OutlineStatus::Pending));
            assert_eq!(state.pending_update.added, second_update.added);
        });

        // Wait for the in-flight recomputation -- which then applies the merged update as a
        // follow-up recomputation -- to fully settle.
        let mut waited = Duration::ZERO;
        loop {
            let is_complete = outlines_handle.read(&app, |outlines, _| {
                matches!(
                    outlines.outlines.get(&repo_path).map(|state| &state.status),
                    Some(OutlineStatus::Complete(_))
                )
            });
            if is_complete {
                break;
            }
            assert!(
                waited < Duration::from_secs(10),
                "recomputation never completed"
            );
            Timer::after(Duration::from_millis(10)).await;
            waited += Duration::from_millis(10);
        }

        outlines_handle.read(&app, |outlines, _| {
            let state = outlines.outlines.get(&repo_path).unwrap();
            assert!(state.pending_update.is_empty());
            let OutlineStatus::Complete(outline) = &state.status else {
                panic!("expected Complete status");
            };
            // The baseline file plus both concurrently-submitted updates are all reflected.
            assert_eq!(outline.file_count(), 3);
        });
    });
}
