use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::executor::block_on;
use futures::future::{self, AbortHandle};
use warp_harness_usage::{CaptureDiagnostics, JsonlDiagnostics, JsonlReadStatus, extract_claude};
use warpui::r#async::executor::Background;

use super::*;
use crate::server::server_api::ServerApiProvider;

fn task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440000".parse().unwrap()
}

fn reporter(client: Arc<ServerApi>) -> UsageReporter {
    let reporter = UsageReporter::default();
    reporter.initialize(
        client,
        Some(task_id()),
        Some(HarnessUsageCapability { execution_id: 41 }),
    );
    reporter
}

fn request(sequence: i64) -> HarnessUsageRequest {
    CaptureIdentity {
        execution_id: 41,
        sequence,
    }
    .build_usage_request(
        Utc.with_ymd_and_hms(2026, 9, 10, 12, 0, 0).unwrap(),
        extract_claude(
            "root",
            &[],
            [],
            &CaptureDiagnostics {
                root: JsonlDiagnostics {
                    status: JsonlReadStatus::Readable,
                    ..Default::default()
                },
                ..Default::default()
            },
        ),
    )
    .unwrap()
}

#[test]
fn retries_keep_exact_request_and_exhaust_without_rearming() {
    let request = request(1);
    let bodies = RefCell::new(Vec::new());
    let delays = RefCell::new(Vec::new());
    let result = block_on(publish_with_retry(
        &request,
        |request| {
            bodies
                .borrow_mut()
                .push(serde_json::to_vec(request).unwrap());
            future::ready(Err(HarnessUsageError::new(
                HarnessUsageErrorKind::Retryable,
            )))
        },
        |delay| {
            delays.borrow_mut().push(delay);
            future::ready(())
        },
    ));

    assert_eq!(result.unwrap_err().kind, HarnessUsageErrorKind::Retryable);
    let bodies = bodies.into_inner();
    assert_eq!(bodies.len(), 3);
    assert!(bodies.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(delays.into_inner().len(), 2);
}

#[test]
fn permanent_errors_and_long_retry_after_stop_immediately() {
    for error in [
        HarnessUsageError::new(HarnessUsageErrorKind::Unauthorized),
        HarnessUsageError::new(HarnessUsageErrorKind::Conflict),
        HarnessUsageError::new(HarnessUsageErrorKind::InvalidReport),
        HarnessUsageError {
            kind: HarnessUsageErrorKind::Retryable,
            status: None,
            retry_after: Some(Duration::from_secs(60)),
        },
    ] {
        let calls = RefCell::new(0);
        let result = block_on(publish_with_retry(
            &request(1),
            |_| {
                *calls.borrow_mut() += 1;
                future::ready(Err(error.clone()))
            },
            |_| async { panic!("must not retry") },
        ));
        assert_eq!(result.unwrap_err(), error);
        assert_eq!(*calls.borrow(), 1);
    }
}

#[test]
fn capture_allocation_preserves_pending_and_cannot_change_execution() {
    let client = ServerApiProvider::new_for_test().get();
    let reporter = reporter(client.clone());
    {
        let mut state = reporter.state.lock();
        state.sequence = 1;
        state.pending = Some(request(1));
    }

    let identity = reporter.begin_capture().unwrap();
    reporter.initialize(
        client,
        Some(task_id()),
        Some(HarnessUsageCapability { execution_id: 42 }),
    );

    assert_eq!((identity.execution_id, identity.sequence), (41, 2));
    assert_eq!(
        reporter
            .state
            .lock()
            .pending
            .as_ref()
            .unwrap()
            .capture_sequence,
        1
    );
    assert_eq!(reporter.begin_capture().unwrap().execution_id, 41);
}

#[test]
fn active_publication_allows_capture_and_keeps_only_the_latest_pending_request() {
    let reporter = reporter(ServerApiProvider::new_for_test().get());
    let (abort, _) = AbortHandle::new_pair();
    let (_done, receiver) = oneshot::channel();
    reporter.state.lock().active = Some(ActivePublisher {
        abort,
        done: receiver.shared(),
    });
    let background = Background::default();

    reporter.stage_request(request(2), &background);
    reporter.stage_request(request(3), &background);

    assert_eq!(
        reporter
            .state
            .lock()
            .pending
            .as_ref()
            .unwrap()
            .capture_sequence,
        3
    );
    assert!(reporter.begin_capture().is_some());
}

#[test]
fn interrupted_drain_cancels_active_and_pending_publication() {
    let reporter = reporter(ServerApiProvider::new_for_test().get());
    let (abort, _) = AbortHandle::new_pair();
    let (_done, receiver) = oneshot::channel();
    let mut state = reporter.state.lock();
    state.pending = Some(request(2));
    state.active = Some(ActivePublisher {
        abort,
        done: receiver.shared(),
    });
    drop(state);

    assert!(
        reporter
            .close_and_drain(Duration::from_secs(5))
            .now_or_never()
            .is_none()
    );
    let state = reporter.state.lock();
    assert!(state.active.is_none());
    assert!(state.pending.is_none());
}
