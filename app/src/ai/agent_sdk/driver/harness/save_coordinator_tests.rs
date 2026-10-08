use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, SystemTime};

use anyhow::{Result, anyhow};
use futures::channel::oneshot;
use futures::{FutureExt as _, future};
use instant::Instant;
use parking_lot::Mutex;
use warpui::r#async::FutureExt as _;
use warpui::r#async::executor::Background;

use super::{SaveCoordinator, SaveOperation, remaining_final_save_budget};
use crate::ai::agent_sdk::driver::harness::SavePoint;
use crate::ai::agent_sdk::driver::harness::harness_persistence::save_transcript_and_block;
use crate::ai::agent_sdk::driver::harness::transcript_persistence::UploadedTranscriptUsage;
#[tokio::test]
async fn fast_saves_share_a_throttle_and_capture_latest_pending_state() {
    let background = Background::default();
    let coordinator = SaveCoordinator::default();
    let current = Arc::new(Mutex::new(0));
    let (saved, saves) = async_channel::unbounded();
    let captured = current.clone();
    coordinator.set_worker_operation(Arc::new(move |point| {
        let saved = saved.clone();
        let captured = captured.clone();
        Box::pin(async move {
            let snapshot = *captured.lock();
            saved.send((point, snapshot, Instant::now())).await?;
            Ok(())
        })
    }));

    let first = Instant::now();
    coordinator.enqueue(SavePoint::Periodic, &background);
    let (point, snapshot, _) = saves.recv().await.unwrap();
    assert_eq!((point, snapshot), (SavePoint::Periodic, 0));
    coordinator.enqueue(SavePoint::PostTurn, &background);
    coordinator.enqueue(SavePoint::Periodic, &background);
    coordinator.enqueue(SavePoint::PostTurn, &background);
    *current.lock() = 3;
    let (point, snapshot, second) = saves
        .recv()
        .with_timeout(Duration::from_secs(35))
        .await
        .unwrap()
        .unwrap();
    assert!(second.duration_since(first) >= Duration::from_secs(30));
    assert_eq!((point, snapshot), (SavePoint::PostTurn, 3));
    assert!(saves.is_empty());
}

#[tokio::test]
async fn final_save_waits_for_throttle_and_supersedes_pending_capture() {
    let background = Background::default();
    let coordinator = SaveCoordinator::default();
    let (saved, saves) = async_channel::unbounded();
    coordinator.set_worker_operation(Arc::new(move |point| {
        let saved = saved.clone();
        Box::pin(async move {
            saved.send(point).await?;
            Ok(())
        })
    }));
    let started = Instant::now();
    coordinator.enqueue(SavePoint::PostTurn, &background);
    assert_eq!(saves.recv().await.unwrap(), SavePoint::PostTurn);
    coordinator.enqueue(SavePoint::Periodic, &background);

    coordinator
        .finalize(
            async {
                assert!(started.elapsed() >= Duration::from_secs(30));
                Ok(())
            },
            future::ready(()),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    assert!(saves.is_empty());
    coordinator.enqueue(SavePoint::PostTurn, &background);
    assert!(saves.is_empty());
}

#[tokio::test]
async fn insufficient_final_budget_does_not_bypass_throttle_or_rearm() {
    let background = Background::default();
    let coordinator = SaveCoordinator::default();
    let (saved, saves) = async_channel::unbounded();
    coordinator.set_worker_operation(Arc::new(move |_| {
        let saved = saved.clone();
        Box::pin(async move {
            saved.send(()).await?;
            Ok(())
        })
    }));
    coordinator.enqueue(SavePoint::Periodic, &background);
    saves.recv().await.unwrap();
    let captured = AtomicBool::new(false);

    assert!(
        coordinator
            .finalize(
                async {
                    captured.store(true, Ordering::SeqCst);
                    Ok(())
                },
                future::ready(()),
                Duration::from_secs(1),
            )
            .await
            .is_err()
    );
    assert!(
        coordinator
            .finalize(
                future::pending::<Result<()>>(),
                future::ready(()),
                Duration::from_secs(60),
            )
            .now_or_never()
            .unwrap()
            .is_err()
    );
    assert!(!captured.load(Ordering::SeqCst));
}

#[tokio::test]
async fn metrics_timeout_preserves_completed_persistence_and_drops_publication() {
    let coordinator = SaveCoordinator::default();
    let (release, released) = oneshot::channel::<()>();
    let result = coordinator
        .finalize(
            future::ready(Ok(())),
            async {
                let _ = released.await;
            },
            Duration::from_millis(10),
        )
        .await;
    assert!(result.is_ok());
    assert!(release.send(()).is_err());
    assert!(
        coordinator
            .finalize(
                future::pending::<Result<()>>(),
                future::pending(),
                Duration::from_secs(30),
            )
            .now_or_never()
            .unwrap()
            .is_ok()
    );
}

#[tokio::test]
async fn coalesces_saves_without_blocking_other_work() {
    let background = Background::default();
    let coordinator = SaveCoordinator::default();
    let saved = Arc::new(Mutex::new(Vec::new()));
    let (started, starts) = async_channel::unbounded();
    let (completed, completions) = async_channel::unbounded();
    let (release, releases) = async_channel::unbounded();
    let recorded = saved.clone();
    let operation: SaveOperation = Arc::new(move |point| {
        let started = started.clone();
        let releases = releases.clone();
        let recorded = recorded.clone();
        let completed = completed.clone();
        Box::pin(async move {
            started.send(point).await?;
            releases.recv().await?;
            recorded.lock().push(point);
            completed.send(()).await?;
            Ok(())
        })
    });

    coordinator.set_worker_operation(operation);
    coordinator.enqueue(SavePoint::Periodic, &background);
    assert_eq!(starts.recv().await.unwrap(), SavePoint::Periodic);
    coordinator.enqueue(SavePoint::PostTurn, &background);
    coordinator.enqueue(SavePoint::Periodic, &background);
    coordinator.enqueue(SavePoint::PostTurn, &background);
    let (ping, pong) = oneshot::channel();
    background
        .spawn(async move { ping.send(()).unwrap() })
        .detach();
    pong.with_timeout(Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    assert!(starts.is_empty());

    release.send(()).await.unwrap();
    completions.recv().await.unwrap();
    assert_eq!(starts.recv().await.unwrap(), SavePoint::PostTurn);
    release.send(()).await.unwrap();
    completions.recv().await.unwrap();
    assert_eq!(*saved.lock(), [SavePoint::Periodic, SavePoint::PostTurn]);
    assert!(starts.is_empty());
}

#[tokio::test]
async fn block_failure_does_not_cancel_raw_transcript() {
    let uploaded = AtomicBool::new(false);
    let (release, released) = oneshot::channel();
    let result = save_transcript_and_block(
        async {
            released.await?;
            uploaded.store(true, Ordering::SeqCst);
            Ok(UploadedTranscriptUsage::empty())
        },
        async {
            release.send(()).unwrap();
            Err(anyhow!("block unavailable"))
        },
    )
    .await
    .into_result();

    assert!(uploaded.load(Ordering::SeqCst));
    assert!(result.is_err());
}

#[tokio::test]
async fn raw_failure_does_not_cancel_block_snapshot() {
    let uploaded = AtomicBool::new(false);
    let (release, released) = oneshot::channel();
    let result = save_transcript_and_block(
        async {
            release.send(()).unwrap();
            Err(anyhow!("raw unavailable"))
        },
        async {
            released.await?;
            uploaded.store(true, Ordering::SeqCst);
            Ok(())
        },
    )
    .await
    .into_result();

    assert!(uploaded.load(Ordering::SeqCst));
    assert!(result.is_err());
}

#[tokio::test]
async fn simultaneous_failures_preserve_both_errors() {
    let error = save_transcript_and_block(
        future::ready(Err(anyhow!("raw unavailable"))),
        future::ready(Err(anyhow!("block unavailable"))),
    )
    .await
    .into_result()
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("raw unavailable"));
    assert!(message.contains("block unavailable"));
}

#[tokio::test]
async fn cancelled_blocking_capture_cannot_upload_after_final_save() {
    let background = Background::default();
    let coordinator = SaveCoordinator::default();
    let uploaded = Arc::new(Mutex::new(Vec::new()));
    let captured_uploads = uploaded.clone();
    let (started, start) = oneshot::channel();
    let (read_done, read_finished) = oneshot::channel();
    let (release, wait_for_release) = mpsc::channel();
    let read = Mutex::new(Some((started, read_done, wait_for_release)));
    let operation: SaveOperation = Arc::new(move |_| {
        let (started, read_done, wait_for_release) = read.lock().take().unwrap();
        let uploaded = captured_uploads.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                started.send(()).unwrap();
                wait_for_release.recv().unwrap();
                read_done.send(()).unwrap();
            })
            .await?;
            uploaded.lock().push("stale");
            Ok(())
        })
    });
    coordinator.set_worker_operation(operation);
    coordinator.enqueue(SavePoint::Periodic, &background);
    start.await.unwrap();
    coordinator.enqueue(SavePoint::PostTurn, &background);

    coordinator
        .finalize(
            async {
                uploaded.lock().push("final");
                Ok(())
            },
            future::ready(()),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    coordinator.enqueue(SavePoint::PostTurn, &background);
    release.send(()).unwrap();
    read_finished.await.unwrap();

    assert_eq!(*uploaded.lock(), ["final"]);
}

#[tokio::test]
async fn expired_final_deadline_never_starts_or_rearms_a_save() {
    let coordinator = SaveCoordinator::default();
    let captured = AtomicBool::new(false);
    assert!(
        coordinator
            .finalize(
                async {
                    captured.store(true, Ordering::SeqCst);
                    Ok(())
                },
                future::ready(()),
                Duration::ZERO,
            )
            .await
            .is_err()
    );
    assert!(
        coordinator
            .finalize(
                async {
                    captured.store(true, Ordering::SeqCst);
                    Ok(())
                },
                future::ready(()),
                Duration::from_secs(30),
            )
            .await
            .is_err()
    );
    assert!(!captured.load(Ordering::SeqCst));
}

#[tokio::test]
async fn final_timeout_cancels_future_before_returning() {
    let coordinator = SaveCoordinator::default();
    let (release, released) = oneshot::channel::<()>();
    let uploaded = AtomicBool::new(false);
    assert!(
        coordinator
            .finalize(
                async {
                    released.await?;
                    uploaded.store(true, Ordering::SeqCst);
                    Ok(())
                },
                future::ready(()),
                Duration::from_millis(10),
            )
            .await
            .is_err()
    );
    assert!(release.send(()).is_err());
    assert!(!uploaded.load(Ordering::SeqCst));
}

#[tokio::test]
async fn interrupted_finalizer_still_joins_the_cancelled_worker() {
    let background = Background::default();
    let coordinator = SaveCoordinator::default();
    let (started, start) = oneshot::channel();
    let (release, released) = oneshot::channel::<()>();
    let current = Mutex::new(Some((started, released)));
    let operation: SaveOperation = Arc::new(move |_| {
        let (started, released) = current.lock().take().unwrap();
        Box::pin(async move {
            started.send(()).unwrap();
            released.await?;
            Ok(())
        })
    });
    coordinator.set_worker_operation(operation);
    coordinator.enqueue(SavePoint::Periodic, &background);
    start.await.unwrap();
    assert!(
        coordinator
            .finalize(
                future::pending::<Result<()>>(),
                future::ready(()),
                Duration::from_secs(60)
            )
            .now_or_never()
            .is_none()
    );

    coordinator
        .finalize(
            async {
                assert!(release.send(()).is_err());
                Ok(())
            },
            future::ready(()),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn final_failure_is_retained_without_repeating_writes() {
    let coordinator = SaveCoordinator::default();
    let result = coordinator
        .finalize(
            async { Err(anyhow!("upload failed")) },
            future::ready(()),
            Duration::from_secs(5),
        )
        .await;
    assert!(result.is_err());
    assert!(
        coordinator
            .finalize(
                future::pending::<Result<()>>(),
                future::ready(()),
                Duration::from_secs(5)
            )
            .now_or_never()
            .unwrap()
            .is_err()
    );
}

#[test]
fn final_budget_respects_earlier_sandbox_deadline() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    assert_eq!(
        remaining_final_save_budget(now, None),
        Duration::from_secs(60)
    );
    assert_eq!(
        remaining_final_save_budget(now, Some(now + Duration::from_secs(10))),
        Duration::from_secs(10)
    );
    assert_eq!(
        remaining_final_save_budget(now, Some(now + Duration::from_secs(90))),
        Duration::from_secs(60)
    );
    assert_eq!(
        remaining_final_save_budget(now, Some(now - Duration::from_secs(1))),
        Duration::ZERO
    );
}
