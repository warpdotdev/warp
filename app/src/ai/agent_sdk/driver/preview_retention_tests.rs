use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::channel::oneshot;
use futures::executor::block_on;
use futures::poll;
use serde_json::json;

use super::super::IdleTimeoutSender;
use super::*;

fn response(preview_count: usize) -> ListRunPreviewsResponse {
    serde_json::from_value(json!({
        "execution_id": 42,
        "previews": vec![json!({}); preview_count],
    }))
    .unwrap()
}

#[test]
fn preview_retention_rejects_malformed_launch_execution_id() {
    let _flag = FeatureFlag::PreviewUrlsMcp.override_enabled(true);
    let task_id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    for raw in ["", "not-an-id", "0", "-1", "9223372036854775808"] {
        assert!(
            PreviewRetention::new(
                task_id,
                Some(raw),
                Arc::new(crate::server::server_api::ai::MockAIClient::new()),
                Arc::new(Background::new(1, |_| "preview-retention-test".into())),
            )
            .is_none()
        );
    }
}
#[test]
fn preview_retention_disabled_without_preview_urls_flag() {
    let _flag = FeatureFlag::PreviewUrlsMcp.override_enabled(false);
    let task_id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();

    assert!(
        PreviewRetention::new(
            task_id,
            Some("42"),
            Arc::new(crate::server::server_api::ai::MockAIClient::new()),
            Arc::new(Background::new(1, |_| "preview-retention-test".into())),
        )
        .is_none()
    );
}

#[test]
fn preview_retention_enabled_with_preview_urls_flag() {
    let _flag = FeatureFlag::PreviewUrlsMcp.override_enabled(true);
    let task_id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();

    assert!(
        PreviewRetention::new(
            task_id,
            Some("42"),
            Arc::new(crate::server::server_api::ai::MockAIClient::new()),
            Arc::new(Background::new(1, |_| "preview-retention-test".into())),
        )
        .is_some()
    );
}

#[test]
#[cfg(not(target_family = "wasm"))]
fn preview_retention_only_applies_to_namespace() {
    assert!(!is_namespace_sandbox(None));
    assert!(!is_namespace_sandbox(Some(IsolationPlatformType::Docker)));
    assert!(!is_namespace_sandbox(Some(
        IsolationPlatformType::Kubernetes
    )));
    assert!(is_namespace_sandbox(Some(IsolationPlatformType::Namespace)));
    assert!(!is_namespace_sandbox(Some(
        IsolationPlatformType::DockerSandbox
    )));
}

fn retention(
    check: impl Fn() -> BoxFuture<'static, Option<Duration>> + Send + Sync + 'static,
    wait: impl Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync + 'static,
) -> PreviewRetention {
    PreviewRetention {
        background: Arc::new(Background::new(1, |_| "preview-retention-test".into())),
        check: Arc::new(check),
        wait: Arc::new(wait),
    }
}

#[test]
fn preview_retention_uses_server_filtered_presence_and_fixed_wait() {
    for (preview_count, expected) in [
        (0, None),
        (1, Some(Duration::from_secs(60))),
        (3, Some(Duration::from_secs(60))),
    ] {
        assert_eq!(
            retention_delay(&response(preview_count), Some(42)),
            expected
        );
    }
    let mut no_execution = response(1);
    no_execution.execution_id = None;
    assert_eq!(retention_delay(&no_execution, None), None);
    for execution_id in [Some(0), Some(-1), Some(43)] {
        let mut other_execution = response(1);
        other_execution.execution_id = execution_id;
        assert_eq!(retention_delay(&other_execution, Some(42)), None);
    }
    assert_eq!(
        retention_delay(&response(1), None),
        Some(Duration::from_secs(60))
    );
}

#[test]
fn preview_retention_rechecks_after_wait_before_commit() {
    block_on(async {
        let (tx, mut rx) = oneshot::channel();
        let committed = Arc::new(AtomicUsize::new(0));
        let count = committed.clone();
        let sender = IdleTimeoutSender::new(tx).with_on_commit(move || {
            count.fetch_add(1, Ordering::SeqCst);
        });
        let checks = Arc::new(Mutex::new(VecDeque::from([
            Some(Duration::from_secs(60)),
            None,
        ])));
        let (release, gate) = oneshot::channel();
        let gate = Mutex::new(Some(gate));
        let retention = retention(
            move || {
                let result = checks.lock().unwrap().pop_front().unwrap();
                Box::pin(async move { result })
            },
            move |duration| {
                assert_eq!(duration, Duration::from_secs(60));
                let gate = gate.lock().unwrap().take().unwrap();
                Box::pin(async move {
                    gate.await.unwrap();
                })
            },
        );
        let mut finish = Box::pin(sender.finish_retained_idle(0, 7, retention));
        assert!(poll!(finish.as_mut()).is_pending());
        assert_eq!(rx.try_recv().unwrap(), None);
        assert_eq!(committed.load(Ordering::SeqCst), 0);
        release.send(()).unwrap();
        finish.await;
        assert_eq!(rx.try_recv().unwrap(), Some(7));
        assert_eq!(committed.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn preview_retention_followup_invalidates_in_flight_response() {
    for response in [None, Some(Duration::from_secs(60))] {
        block_on(async {
            let (tx, mut rx) = oneshot::channel::<i32>();
            let committed = Arc::new(AtomicUsize::new(0));
            let count = committed.clone();
            let sender = IdleTimeoutSender::new(tx).with_on_commit(move || {
                count.fetch_add(1, Ordering::SeqCst);
            });
            let (release, gate) = oneshot::channel();
            let gate = Mutex::new(Some(gate));
            let retention = retention(
                move || {
                    let gate = gate.lock().unwrap().take().unwrap();
                    Box::pin(async move { gate.await.unwrap() })
                },
                |_| panic!("stale response must not retain a superseded turn"),
            );
            let mut finish = Box::pin(sender.finish_retained_idle(0, 1, retention));
            assert!(poll!(finish.as_mut()).is_pending());
            sender.cancel_idle_timeout();
            release.send(response).unwrap();
            finish.await;
            assert_eq!(rx.try_recv().unwrap(), None);
            assert_eq!(committed.load(Ordering::SeqCst), 0);
            sender.commit_idle_timeout(0, 2);
            assert_eq!(rx.try_recv().unwrap(), None);
            sender.end_run_now(3);
            assert_eq!(rx.try_recv().unwrap(), Some(3));
        });
    }
}

#[test]
fn preview_retention_explicit_exit_bypasses_in_flight_query() {
    block_on(async {
        let (tx, mut rx) = oneshot::channel();
        let sender = IdleTimeoutSender::new(tx);
        let retention = retention(|| Box::pin(futures::future::pending()), |_| unreachable!());
        let mut finish = Box::pin(sender.finish_retained_idle(0, 1, retention));
        assert!(poll!(finish.as_mut()).is_pending());
        sender.end_run_now(2);
        assert_eq!(rx.try_recv().unwrap(), Some(2));
    });
}

#[test]
fn preview_retention_none_zero_and_automatic_recovery_bypass_check() {
    for timeout in [None, Some(Duration::ZERO)] {
        let (tx, rx) = oneshot::channel();
        let mut sender = IdleTimeoutSender::new(tx);
        sender.preview_retention = Some(retention(
            || panic!("must bypass check"),
            |_| unreachable!(),
        ));
        sender.complete_with_optional_idle(timeout, 5);
        assert_eq!(block_on(rx).unwrap(), 5);
    }
    let (tx, rx) = oneshot::channel();
    let mut sender = IdleTimeoutSender::new(tx);
    sender.preview_retention = Some(retention(
        || panic!("must bypass check"),
        |_| unreachable!(),
    ));
    sender.end_run_after(Duration::ZERO, 6);
    assert_eq!(block_on(rx).unwrap(), 6);
}

struct NoIdleWait;

impl super::super::IdleWait for NoIdleWait {
    fn wait(&self, _: Duration) {}
}

struct SynchronizedIdleWait(std::sync::Barrier);

impl super::super::IdleWait for SynchronizedIdleWait {
    fn wait(&self, _: Duration) {
        self.0.wait();
    }
}

#[test]
fn preview_retention_zero_failure_window_bypasses_check() {
    let (tx, rx) = oneshot::channel();
    let mut sender = IdleTimeoutSender::new(tx);
    sender.wait = Arc::new(NoIdleWait);
    sender.preview_retention = Some(retention(
        || panic!("zero window must bypass check"),
        |_| unreachable!(),
    ));

    sender.arm_refreshable(Duration::ZERO, 7);

    assert_eq!(block_on(rx).unwrap(), 7);
}

#[test]
fn preview_retention_zero_failure_window_remains_refreshable_without_check() {
    let (tx, mut rx) = oneshot::channel();
    let mut sender = IdleTimeoutSender::new(tx);
    let wait = Arc::new(SynchronizedIdleWait(std::sync::Barrier::new(3)));
    sender.wait = wait.clone();
    sender.preview_retention = Some(retention(
        || panic!("refreshed zero window must bypass check"),
        |_| unreachable!(),
    ));

    sender.arm_refreshable(Duration::ZERO, 7);

    assert_eq!(rx.try_recv().unwrap(), None);
    assert_eq!(sender.refresh(), Some(Duration::ZERO));
    wait.0.wait();
    assert_eq!(block_on(rx).unwrap(), 7);
}

#[test]
fn preview_retention_refreshed_failure_window_checks_before_commit() {
    let (tx, rx) = oneshot::channel();
    let checks = Arc::new(AtomicUsize::new(0));
    let checks_for_commit = checks.clone();
    let mut sender = IdleTimeoutSender::new(tx).with_on_commit(move || {
        assert_eq!(checks_for_commit.load(Ordering::SeqCst), 1);
    });
    let wait = Arc::new(SynchronizedIdleWait(std::sync::Barrier::new(3)));
    sender.wait = wait.clone();
    sender.preview_retention = Some(retention(
        move || {
            checks.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { None })
        },
        |_| unreachable!(),
    ));
    sender.arm_refreshable(Duration::from_secs(10), 7);
    assert_eq!(sender.refresh(), Some(Duration::from_secs(10)));
    wait.0.wait();
    assert_eq!(block_on(rx).unwrap(), 7);
}

#[test]
fn preview_retention_completion_and_failure_idle_check_before_commit() {
    for failure_window in [false, true] {
        let (tx, rx) = oneshot::channel();
        let checks = Arc::new(AtomicUsize::new(0));
        let checks_for_commit = checks.clone();
        let mut sender = IdleTimeoutSender::new(tx).with_on_commit(move || {
            assert_eq!(checks_for_commit.load(Ordering::SeqCst), 1);
        });
        sender.wait = Arc::new(NoIdleWait);
        sender.preview_retention = Some(retention(
            move || {
                checks.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { None })
            },
            |_| unreachable!(),
        ));
        if failure_window {
            sender.arm_refreshable(Duration::from_secs(10), 7);
        } else {
            sender.complete_with_optional_idle(Some(Duration::from_secs(10)), 7);
        }
        assert_eq!(block_on(rx).unwrap(), 7);
    }
}
#[test]
fn preview_retention_query_failure_and_timeout_allow_exit() {
    let failed = futures::future::ready(Err(anyhow::anyhow!("404 older server")));
    assert_eq!(
        block_on(check_previews(failed, None, PREVIEW_CHECK_TIMEOUT)),
        None
    );
    assert_eq!(
        block_on(check_previews(
            futures::future::pending(),
            None,
            Duration::ZERO
        )),
        None
    );
}
