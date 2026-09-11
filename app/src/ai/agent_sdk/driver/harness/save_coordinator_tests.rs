use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, SystemTime};

use anyhow::{Result, anyhow};
use futures::channel::oneshot;
use futures::{FutureExt as _, future};
use parking_lot::Mutex;
use warpui::r#async::FutureExt as _;
use warpui::r#async::executor::Background;

use super::{
    SaveCoordinator, SaveOperation, remaining_final_save_budget, save_transcript_and_block,
};
use crate::ai::agent_sdk::driver::harness::SavePoint;
#[tokio::test]
async fn metrics_timeout_preserves_completed_persistence_and_drops_publication() {
    let coordinator = SaveCoordinator::default();
    let (release, released) = oneshot::channel::<()>();
    let result = coordinator.finish(
        future::ready(Ok(())),
        async { let _ = released.await; },
        Duration::from_millis(10),
    ).await;
    assert!(result.is_ok());
    assert!(release.send(()).is_err());
    assert!(coordinator.finish(
        future::pending::<Result<()>>(),
        future::pending(),
        Duration::from_secs(30),
    ).now_or_never().unwrap().is_ok());
}

#[tokio::test]
async fn coalesces_saves_without_blocking_other_work() {
    let background = Background::default();
    let coordinator = SaveCoordinator::default();
    let saved = Arc::new(Mutex::new(Vec::new()));
    let (started, starts) = async_channel::unbounded();
    let (release, releases) = async_channel::unbounded();
    let recorded = saved.clone();
    let operation: SaveOperation = Arc::new(move |point| {
        let started = started.clone();
        let releases = releases.clone();
        let recorded = recorded.clone();
        Box::pin(async move {
            started.send(point).await?;
            releases.recv().await?;
            recorded.lock().push(point);
            Ok(())
        })
    });

    coordinator.request(SavePoint::Periodic, operation.clone(), &background);
    assert_eq!(starts.recv().await.unwrap(), SavePoint::Periodic);
    coordinator.request(SavePoint::PostTurn, operation.clone(), &background);
    coordinator.request(SavePoint::Periodic, operation.clone(), &background);
    coordinator.request(SavePoint::PostTurn, operation, &background);
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
    assert_eq!(starts.recv().await.unwrap(), SavePoint::PostTurn);
    release.send(()).await.unwrap();
    coordinator
        .finish(
            async {
                saved.lock().push(SavePoint::Final);
                Ok(())
            },
            future::ready(()),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(
        *saved.lock(),
        [SavePoint::Periodic, SavePoint::PostTurn, SavePoint::Final]
    );
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
            Ok(())
        },
        async {
            release.send(()).unwrap();
            Err(anyhow!("block unavailable"))
        },
    )
    .await;

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
    .await;

    assert!(uploaded.load(Ordering::SeqCst));
    assert!(result.is_err());
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
    coordinator.request(SavePoint::Periodic, operation.clone(), &background);
    start.await.unwrap();
    coordinator.request(SavePoint::PostTurn, operation.clone(), &background);

    coordinator
        .finish(
            async {
                uploaded.lock().push("final");
                Ok(())
            },
            future::ready(()),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    coordinator.request(SavePoint::PostTurn, operation, &background);
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
            .finish(
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
            .finish(
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
            .finish(
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
    coordinator.request(SavePoint::Periodic, operation, &background);
    start.await.unwrap();
    assert!(
        coordinator
            .finish(future::pending::<Result<()>>(), future::ready(()), Duration::from_secs(5))
            .now_or_never()
            .is_none()
    );

    coordinator
        .finish(
            async {
                assert!(release.send(()).is_err());
                Ok(())
            },
            future::ready(()),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn final_failure_is_retained_without_repeating_writes() {
    let coordinator = SaveCoordinator::default();
    let result = coordinator
        .finish(
            async { Err(anyhow!("upload failed")) },
            future::ready(()),
            Duration::from_secs(5),
        )
        .await;
    assert!(result.is_err());
    assert!(
        coordinator
            .finish(future::pending::<Result<()>>(), future::ready(()), Duration::from_secs(5))
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
        Duration::from_secs(30)
    );
    assert_eq!(
        remaining_final_save_budget(now, Some(now + Duration::from_secs(10))),
        Duration::from_secs(10)
    );
    assert_eq!(
        remaining_final_save_budget(now, Some(now + Duration::from_secs(60))),
        Duration::from_secs(30)
    );
    assert_eq!(
        remaining_final_save_budget(now, Some(now - Duration::from_secs(1))),
        Duration::ZERO
    );
}
