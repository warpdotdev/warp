use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use instant::Instant;
use mockito::{Matcher, Server};
use tempfile::TempDir;
use tokio::sync::oneshot;
use uuid::Uuid;

use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_sdk::test_support::build_test_http_client;
use crate::ai::artifacts::Artifact;
use crate::server::server_api::harness_support::{
    CommitSnapshotRequest, CommitSnapshotResponse, ReportArtifactResponse, ResolvePromptRequest,
    ResolvedHarnessPrompt, SnapshotUploadRequest, UploadTarget,
};

fn fresh_task_id() -> AmbientAgentTaskId {
    AmbientAgentTaskId::from_str(&Uuid::new_v4().to_string()).unwrap()
}

fn test_background() -> Arc<Background> {
    Arc::new(Background::new(1, |_| {
        "checkpoint-coordinator-test".to_string()
    }))
}

/// Poll `condition` until it's true or `timeout` elapses, then assert it held.
async fn wait_for(mut condition: impl FnMut() -> bool, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !condition() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(condition(), "condition not met within {timeout:?}");
}

/// Writes a single-file declarations JSONL at the default per-task-id path that
/// `run_one_attempt` resolves internally (it has no path-override hook), and removes the
/// per-task directory on drop so parallel tests never collide (each uses a fresh task id).
struct DeclarationsFixture {
    dir: PathBuf,
}

impl DeclarationsFixture {
    fn new(task_id: &AmbientAgentTaskId, declared_file: &Path) -> Self {
        let path = snapshot::resolve_declarations_path(Some(task_id));
        let dir = path.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let mut contents = serde_json::json!({
            "version": 1,
            "kind": "file",
            "path": declared_file.to_string_lossy(),
        })
        .to_string();
        contents.push('\n');
        std::fs::write(&path, contents).unwrap();
        Self { dir }
    }
}

impl Drop for DeclarationsFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A `HarnessSupportClient` whose `get_snapshot_upload_targets` always fails immediately
/// (no HTTP involved), so `run_one_attempt` resolves to `Failed` cheaply. Used for tests that
/// only care about coordinator timing/gating, not upload success. `commit_snapshot` panics:
/// it must never be reached when uploads always fail.
struct FailingUploadTargetsClient {
    http: http_client::Client,
    call_count: AtomicUsize,
}

impl FailingUploadTargetsClient {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            http: build_test_http_client(),
            call_count: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl HarnessSupportClient for FailingUploadTargetsClient {
    async fn create_external_conversation(&self, _format: &str) -> Result<AIConversationId> {
        unimplemented!("not used by the coordinator")
    }
    async fn get_transcript_upload_target(
        &self,
        _conversation_id: &AIConversationId,
    ) -> Result<UploadTarget> {
        unimplemented!("not used by the coordinator")
    }
    async fn get_block_snapshot_upload_target(
        &self,
        _conversation_id: &AIConversationId,
    ) -> Result<UploadTarget> {
        unimplemented!("not used by the coordinator")
    }
    async fn resolve_prompt(
        &self,
        _request: ResolvePromptRequest,
    ) -> Result<ResolvedHarnessPrompt> {
        unimplemented!("not used by the coordinator")
    }
    async fn report_artifact(&self, _artifact: &Artifact) -> Result<ReportArtifactResponse> {
        unimplemented!("not used by the coordinator")
    }
    async fn notify_user(&self, _message: &str) -> Result<()> {
        unimplemented!("not used by the coordinator")
    }
    async fn finish_task(&self, _success: bool, _summary: &str) -> Result<()> {
        unimplemented!("not used by the coordinator")
    }
    async fn report_clean_shutdown(&self) -> Result<()> {
        unimplemented!("not used by the coordinator")
    }
    async fn report_error_shutdown(
        &self,
        _error_category: String,
        _error_message: String,
    ) -> Result<()> {
        unimplemented!("not used by the coordinator")
    }
    async fn get_snapshot_upload_targets(
        &self,
        _request: &SnapshotUploadRequest,
    ) -> Result<Vec<UploadTarget>> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("simulated get_snapshot_upload_targets failure")
    }
    async fn commit_snapshot(
        &self,
        _request: &CommitSnapshotRequest,
    ) -> Result<CommitSnapshotResponse> {
        panic!("commit_snapshot must never be called when uploads always fail");
    }
    async fn fetch_transcript(&self) -> Result<bytes::Bytes> {
        unimplemented!("not used by the coordinator")
    }
    fn http_client(&self) -> &http_client::Client {
        &self.http
    }
}

/// A `HarnessSupportClient` that serves real presigned-URL uploads against a `mockito` server
/// and records call counts, with an optional artificial delay before `commit_snapshot`
/// resolves (used to keep an attempt "in flight" for finalization tests).
struct RecordingClient {
    server_base_url: String,
    http: http_client::Client,
    commit_delay: Duration,
    fail_commit: bool,
    upload_calls: AtomicUsize,
    commit_calls: AtomicUsize,
}

impl RecordingClient {
    fn new(server_base_url: String) -> Arc<Self> {
        Self::with_options(server_base_url, Duration::ZERO, false)
    }

    fn with_commit_delay(server_base_url: String, delay: Duration) -> Arc<Self> {
        Self::with_options(server_base_url, delay, false)
    }

    fn with_options(
        server_base_url: String,
        commit_delay: Duration,
        fail_commit: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            server_base_url,
            http: build_test_http_client(),
            commit_delay,
            fail_commit,
            upload_calls: AtomicUsize::new(0),
            commit_calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl HarnessSupportClient for RecordingClient {
    async fn create_external_conversation(&self, _format: &str) -> Result<AIConversationId> {
        unimplemented!("not used by the coordinator")
    }
    async fn get_transcript_upload_target(
        &self,
        _conversation_id: &AIConversationId,
    ) -> Result<UploadTarget> {
        unimplemented!("not used by the coordinator")
    }
    async fn get_block_snapshot_upload_target(
        &self,
        _conversation_id: &AIConversationId,
    ) -> Result<UploadTarget> {
        unimplemented!("not used by the coordinator")
    }
    async fn resolve_prompt(
        &self,
        _request: ResolvePromptRequest,
    ) -> Result<ResolvedHarnessPrompt> {
        unimplemented!("not used by the coordinator")
    }
    async fn report_artifact(&self, _artifact: &Artifact) -> Result<ReportArtifactResponse> {
        unimplemented!("not used by the coordinator")
    }
    async fn notify_user(&self, _message: &str) -> Result<()> {
        unimplemented!("not used by the coordinator")
    }
    async fn finish_task(&self, _success: bool, _summary: &str) -> Result<()> {
        unimplemented!("not used by the coordinator")
    }
    async fn report_clean_shutdown(&self) -> Result<()> {
        unimplemented!("not used by the coordinator")
    }
    async fn report_error_shutdown(
        &self,
        _error_category: String,
        _error_message: String,
    ) -> Result<()> {
        unimplemented!("not used by the coordinator")
    }
    async fn get_snapshot_upload_targets(
        &self,
        request: &SnapshotUploadRequest,
    ) -> Result<Vec<UploadTarget>> {
        self.upload_calls.fetch_add(1, Ordering::SeqCst);
        Ok(request
            .files
            .iter()
            .map(|f| UploadTarget {
                url: format!("{}/upload/{}", self.server_base_url, f.filename),
                method: "PUT".to_string(),
                headers: HashMap::new(),
                fields: Vec::new(),
            })
            .collect())
    }
    async fn commit_snapshot(
        &self,
        request: &CommitSnapshotRequest,
    ) -> Result<CommitSnapshotResponse> {
        self.commit_calls.fetch_add(1, Ordering::SeqCst);
        if !self.commit_delay.is_zero() {
            tokio::time::sleep(self.commit_delay).await;
        }
        if self.fail_commit {
            anyhow::bail!("simulated commit_snapshot failure");
        }
        Ok(CommitSnapshotResponse {
            generation: request.generation.clone(),
        })
    }
    async fn fetch_transcript(&self) -> Result<bytes::Bytes> {
        unimplemented!("not used by the coordinator")
    }
    fn http_client(&self) -> &http_client::Client {
        &self.http
    }
}

// ------------------------------------------------------------------------------------------------
// Pure helpers.
// ------------------------------------------------------------------------------------------------

#[test]
fn jittered_interval_adds_bounded_nonnegative_jitter() {
    let base = Duration::from_secs(60);
    for _ in 0..50 {
        let jittered = jittered_interval(base, CHECKPOINT_JITTER);
        assert!(jittered >= base, "jitter must never shrink the interval");
        assert!(
            jittered <= base + CHECKPOINT_JITTER,
            "jitter must stay within the configured bound"
        );
    }
}

#[test]
fn jittered_interval_zero_jitter_is_a_no_op() {
    let base = Duration::from_millis(5);
    assert_eq!(jittered_interval(base, Duration::ZERO), base);
}

// ------------------------------------------------------------------------------------------------
// finalize_with_new_attempt / finalize_with_in_flight_attempt.
// ------------------------------------------------------------------------------------------------

#[test]
fn finalize_budget_exceeds_the_attempt_floor_for_the_driver_defaults() {
    // Regression: `AgentDriver::run_snapshot_upload` used to pass `upload_timeout` alone as
    // the finalize budget, but the floor is `script_timeout + upload_timeout`. The
    // `remaining > floor` gate was therefore false for *every* possible configuration, so
    // the final attempt was always skipped -- and since the coordinator owns the whole
    // end-of-run path, that meant no end-of-run snapshot at all.
    let script = snapshot::DEFAULT_DECLARATIONS_SCRIPT_TIMEOUT;
    let upload = snapshot::DEFAULT_SNAPSHOT_UPLOAD_TIMEOUT;
    assert!(
        finalize_budget(script, upload) > script + upload,
        "the budget callers derive must exceed the floor `finalize_with_new_attempt` enforces"
    );
    // The old, broken pairing, asserted explicitly so nobody reintroduces it.
    assert!(
        upload <= script + upload,
        "passing the upload timeout alone can never clear the floor"
    );
}

#[tokio::test]
async fn finalize_with_new_attempt_below_floor_skips_the_attempt_entirely() {
    let client = FailingUploadTargetsClient::new();
    let task_id = fresh_task_id();
    let (ack_tx, ack_rx) = oneshot::channel();
    let request = FinalizeRequest {
        deadline: Instant::now() + Duration::from_millis(50),
        ack: ack_tx,
    };
    // floor = script_timeout + upload_timeout = 2s, comfortably above the 50ms budget.
    finalize_with_new_attempt(
        request,
        client.clone(),
        task_id,
        PathBuf::from("/tmp"),
        None,
        Duration::from_secs(1),
        Duration::from_secs(1),
        None,
    )
    .await;
    assert_eq!(
        client.call_count.load(Ordering::SeqCst),
        0,
        "a below-floor budget must skip the attempt without calling the client at all"
    );
    assert!(ack_rx.await.is_ok(), "ack must still be sent");
}

#[tokio::test]
async fn finalize_with_the_derived_budget_runs_the_attempt() {
    // The companion to the unit test above: a budget built by `finalize_budget` from the
    // same timeouts the coordinator holds must clear the floor and actually commit.
    let mut server = Server::new_async().await;
    let _uploads = server
        .mock("PUT", Matcher::Regex("^/upload/.+$".to_string()))
        .with_status(200)
        .create_async()
        .await;

    let task_id = fresh_task_id();
    let tempdir = TempDir::new().unwrap();
    let file_path = tempdir.path().join("note.txt");
    std::fs::write(&file_path, b"hi").unwrap();
    let _decl = DeclarationsFixture::new(&task_id, &file_path);

    let script_timeout = Duration::from_millis(10);
    let upload_timeout = Duration::from_secs(5);
    let client = RecordingClient::new(server.url());
    let (ack_tx, ack_rx) = oneshot::channel();
    let request = FinalizeRequest {
        deadline: Instant::now() + finalize_budget(script_timeout, upload_timeout),
        ack: ack_tx,
    };
    finalize_with_new_attempt(
        request,
        client.clone(),
        task_id,
        tempdir.path().to_path_buf(),
        None,
        script_timeout,
        upload_timeout,
        None,
    )
    .await;
    assert_eq!(
        client.commit_calls.load(Ordering::SeqCst),
        1,
        "a budget derived from `finalize_budget` must run exactly one final attempt"
    );
    assert!(ack_rx.await.is_ok());
}

#[tokio::test]
async fn finalize_with_new_attempt_above_floor_starts_and_commits() {
    let mut server = Server::new_async().await;
    let _uploads = server
        .mock("PUT", Matcher::Regex("^/upload/.+$".to_string()))
        .with_status(200)
        .create_async()
        .await;

    let task_id = fresh_task_id();
    let tempdir = TempDir::new().unwrap();
    let file_path = tempdir.path().join("note.txt");
    std::fs::write(&file_path, b"hi").unwrap();
    let _decl = DeclarationsFixture::new(&task_id, &file_path);

    let client = RecordingClient::new(server.url());
    let (ack_tx, ack_rx) = oneshot::channel();
    let request = FinalizeRequest {
        deadline: Instant::now() + Duration::from_secs(10),
        ack: ack_tx,
    };
    finalize_with_new_attempt(
        request,
        client.clone(),
        task_id,
        tempdir.path().to_path_buf(),
        None,
        Duration::from_millis(10),
        Duration::from_secs(5),
        None,
    )
    .await;
    assert_eq!(
        client.commit_calls.load(Ordering::SeqCst),
        1,
        "an above-floor budget must run exactly one attempt to completion"
    );
    assert!(ack_rx.await.is_ok());
}

#[tokio::test]
async fn finalize_with_in_flight_attempt_stops_waiting_once_the_budget_elapses() {
    let (result_tx, result_rx) = oneshot::channel::<CheckpointResult>();
    let (ack_tx, ack_rx) = oneshot::channel();
    let request = FinalizeRequest {
        deadline: Instant::now() + Duration::from_millis(50),
        ack: ack_tx,
    };
    let start = Instant::now();
    finalize_with_in_flight_attempt(request, result_rx).await;
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "must stop waiting once the budget elapses, not block indefinitely"
    );
    assert!(
        ack_rx.await.is_ok(),
        "ack must still be sent after timing out"
    );
    drop(result_tx);
}

// ------------------------------------------------------------------------------------------------
// start_attempt: no stranded work when the caller stops waiting.
// ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn start_attempt_runs_to_completion_even_if_the_receiver_is_dropped() {
    let client = FailingUploadTargetsClient::new();
    let task_id = fresh_task_id();
    let tempdir = TempDir::new().unwrap();
    let file_path = tempdir.path().join("note.txt");
    std::fs::write(&file_path, b"hi").unwrap();
    let _decl = DeclarationsFixture::new(&task_id, &file_path);

    let background = test_background();
    let rx = start_attempt(
        client.clone(),
        task_id,
        tempdir.path().to_path_buf(),
        None,
        Duration::from_secs(5),
        Duration::from_secs(5),
        None,
        &background,
    );
    drop(rx); // The caller stops waiting immediately; the spawned task must still run.

    wait_for(
        || client.call_count.load(Ordering::SeqCst) >= 1,
        Duration::from_secs(2),
    )
    .await;
}

// ------------------------------------------------------------------------------------------------
// Full coordinator_loop state-machine behavior, via the test-only boundary-check injection.
// ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn coordinator_defers_until_safe_boundary_then_commits() {
    let mut server = Server::new_async().await;
    let _uploads = server
        .mock("PUT", Matcher::Regex("^/upload/.+$".to_string()))
        .with_status(200)
        .create_async()
        .await;

    let task_id = fresh_task_id();
    let tempdir = TempDir::new().unwrap();
    let file_path = tempdir.path().join("note.txt");
    std::fs::write(&file_path, b"hi").unwrap();
    let _decl = DeclarationsFixture::new(&task_id, &file_path);

    let client = RecordingClient::new(server.url());
    let boundary_calls = Arc::new(AtomicUsize::new(0));
    let boundary_calls_for_closure = boundary_calls.clone();
    // False for the first two checks (simulating a mutating tool in flight / not yet
    // WaitingForEvents), true from the third check onward.
    let boundary_check: BoundaryCheck = Arc::new(move || {
        let calls = boundary_calls_for_closure.clone();
        Box::pin(async move {
            if calls.fetch_add(1, Ordering::SeqCst) >= 2 {
                Boundary::Safe
            } else {
                Boundary::Busy
            }
        })
    });

    let handle = CheckpointCoordinatorHandle::new_for_test(
        client.clone(),
        task_id,
        tempdir.path().to_path_buf(),
        boundary_check,
        Duration::from_millis(5),
        Duration::from_secs(5),
        Duration::from_secs(5),
        test_background(),
    );

    wait_for(
        || client.commit_calls.load(Ordering::SeqCst) >= 1,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        boundary_calls.load(Ordering::SeqCst) >= 3,
        "expected the coordinator to keep polling until the boundary became safe"
    );
    drop(handle);
}

#[tokio::test]
async fn coordinator_returns_to_idle_and_waits_a_full_interval_before_the_next_attempt() {
    let mut server = Server::new_async().await;
    let _uploads = server
        .mock("PUT", Matcher::Regex("^/upload/.+$".to_string()))
        .with_status(200)
        .create_async()
        .await;

    let task_id = fresh_task_id();
    let tempdir = TempDir::new().unwrap();
    let file_path = tempdir.path().join("note.txt");
    std::fs::write(&file_path, b"hi").unwrap();
    let _decl = DeclarationsFixture::new(&task_id, &file_path);

    let client = RecordingClient::new(server.url());
    let always_safe: BoundaryCheck = Arc::new(|| Box::pin(async { Boundary::Safe }));
    let interval = Duration::from_millis(400);

    let handle = CheckpointCoordinatorHandle::new_for_test(
        client.clone(),
        task_id,
        tempdir.path().to_path_buf(),
        always_safe,
        interval,
        Duration::from_secs(5),
        Duration::from_secs(5),
        test_background(),
    );

    wait_for(
        || client.commit_calls.load(Ordering::SeqCst) >= 1,
        Duration::from_secs(5),
    )
    .await;

    // Well within `interval` after the first commit, no second attempt should have started.
    tokio::time::sleep(interval / 4).await;
    assert_eq!(
        client.commit_calls.load(Ordering::SeqCst),
        1,
        "success must return to Idle and wait a full interval before the next attempt"
    );

    // After the interval (plus the immediate safe-boundary check) elapses, a second attempt runs.
    wait_for(
        || client.commit_calls.load(Ordering::SeqCst) >= 2,
        Duration::from_secs(5),
    )
    .await;
    drop(handle);
}

#[tokio::test]
async fn coordinator_finalize_awaits_the_in_flight_attempt_without_a_second_commit() {
    let mut server = Server::new_async().await;
    let _uploads = server
        .mock("PUT", Matcher::Regex("^/upload/.+$".to_string()))
        .with_status(200)
        .create_async()
        .await;

    let task_id = fresh_task_id();
    let tempdir = TempDir::new().unwrap();
    let file_path = tempdir.path().join("note.txt");
    std::fs::write(&file_path, b"hi").unwrap();
    let _decl = DeclarationsFixture::new(&task_id, &file_path);

    // A slow commit keeps the periodic attempt "in flight" long enough for finalize() to race
    // in while it's still running.
    let client = RecordingClient::with_commit_delay(server.url(), Duration::from_millis(500));
    let always_safe: BoundaryCheck = Arc::new(|| Box::pin(async { Boundary::Safe }));

    let handle = CheckpointCoordinatorHandle::new_for_test(
        client.clone(),
        task_id,
        tempdir.path().to_path_buf(),
        always_safe,
        Duration::from_millis(5),
        Duration::from_secs(5),
        Duration::from_secs(5),
        test_background(),
    );

    // Give the coordinator time to enter InFlight and call commit_snapshot (which is now
    // sleeping for commit_delay), but not enough time for it to resolve.
    wait_for(
        || client.commit_calls.load(Ordering::SeqCst) >= 1,
        Duration::from_secs(2),
    )
    .await;

    // Finalize with ample budget: it must await the already-in-flight attempt rather than
    // starting a new one.
    handle.finalize(Duration::from_secs(5)).await;

    assert_eq!(
        client.commit_calls.load(Ordering::SeqCst),
        1,
        "finalization must never start a second attempt while one is already in flight"
    );
}

#[tokio::test]
async fn coordinator_stops_when_the_driver_is_gone() {
    // Regression: `is_safe_boundary` used to map a dropped `AgentDriver` to "safe", so the
    // loop kept gathering and uploading the whole workspace every interval forever. Nothing
    // else ever stops it -- `run_snapshot_upload` returns without finalizing when the
    // spawner round trip fails, which is the same condition.
    let client = FailingUploadTargetsClient::new();
    let task_id = fresh_task_id();
    let tempdir = TempDir::new().unwrap();
    let file_path = tempdir.path().join("note.txt");
    std::fs::write(&file_path, b"hi").unwrap();
    let _decl = DeclarationsFixture::new(&task_id, &file_path);

    let boundary_calls = Arc::new(AtomicUsize::new(0));
    let boundary_calls_for_closure = boundary_calls.clone();
    let driver_gone: BoundaryCheck = Arc::new(move || {
        let calls = boundary_calls_for_closure.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Boundary::DriverGone
        })
    });

    let handle = CheckpointCoordinatorHandle::new_for_test(
        client.clone(),
        task_id,
        tempdir.path().to_path_buf(),
        driver_gone,
        Duration::from_millis(5),
        Duration::from_secs(5),
        Duration::from_secs(5),
        test_background(),
    );

    wait_for(
        || boundary_calls.load(Ordering::SeqCst) >= 1,
        Duration::from_secs(5),
    )
    .await;
    // Well past several intervals: the loop must have returned rather than kept ticking.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        boundary_calls.load(Ordering::SeqCst),
        1,
        "a gone driver must stop the loop, not be treated as a safe boundary"
    );
    assert_eq!(
        client.call_count.load(Ordering::SeqCst),
        0,
        "no attempt may run once the driver is gone"
    );
    drop(handle);
}

#[tokio::test]
async fn coordinator_finalize_while_waiting_for_a_safe_boundary_still_runs_an_attempt() {
    // The `Due` state polls indefinitely while the conversation is busy. A finalize request
    // arriving in that window must be serviced (with a new attempt, since none is in flight)
    // rather than waiting for a boundary that may never come.
    let mut server = Server::new_async().await;
    let _uploads = server
        .mock("PUT", Matcher::Regex("^/upload/.+$".to_string()))
        .with_status(200)
        .create_async()
        .await;

    let task_id = fresh_task_id();
    let tempdir = TempDir::new().unwrap();
    let file_path = tempdir.path().join("note.txt");
    std::fs::write(&file_path, b"hi").unwrap();
    let _decl = DeclarationsFixture::new(&task_id, &file_path);

    let boundary_calls = Arc::new(AtomicUsize::new(0));
    let boundary_calls_for_closure = boundary_calls.clone();
    let never_safe: BoundaryCheck = Arc::new(move || {
        let calls = boundary_calls_for_closure.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Boundary::Busy
        })
    });

    let script_timeout = Duration::from_millis(10);
    let upload_timeout = Duration::from_secs(5);
    let client = RecordingClient::new(server.url());
    let handle = CheckpointCoordinatorHandle::new_for_test(
        client.clone(),
        task_id,
        tempdir.path().to_path_buf(),
        never_safe,
        Duration::from_millis(5),
        script_timeout,
        upload_timeout,
        test_background(),
    );

    // Wait until the coordinator is definitely parked in `Due`.
    wait_for(
        || boundary_calls.load(Ordering::SeqCst) >= 1,
        Duration::from_secs(5),
    )
    .await;

    handle
        .finalize(finalize_budget(script_timeout, upload_timeout))
        .await;

    assert_eq!(
        client.commit_calls.load(Ordering::SeqCst),
        1,
        "finalize arriving during boundary polling must still run one final attempt"
    );
}

#[tokio::test]
async fn coordinator_stops_running_attempts_after_finalize() {
    // `finalize` is documented to stop the coordinator. Prove the loop actually returns
    // instead of continuing to tick in the background after shutdown has moved on.
    let mut server = Server::new_async().await;
    let _uploads = server
        .mock("PUT", Matcher::Regex("^/upload/.+$".to_string()))
        .with_status(200)
        .create_async()
        .await;

    let task_id = fresh_task_id();
    let tempdir = TempDir::new().unwrap();
    let file_path = tempdir.path().join("note.txt");
    std::fs::write(&file_path, b"hi").unwrap();
    let _decl = DeclarationsFixture::new(&task_id, &file_path);

    let script_timeout = Duration::from_millis(10);
    let upload_timeout = Duration::from_secs(5);
    let client = RecordingClient::new(server.url());
    let always_safe: BoundaryCheck = Arc::new(|| Box::pin(async { Boundary::Safe }));
    let interval = Duration::from_millis(50);

    let handle = CheckpointCoordinatorHandle::new_for_test(
        client.clone(),
        task_id,
        tempdir.path().to_path_buf(),
        always_safe,
        interval,
        script_timeout,
        upload_timeout,
        test_background(),
    );

    handle
        .finalize(finalize_budget(script_timeout, upload_timeout))
        .await;
    let after_finalize = client.commit_calls.load(Ordering::SeqCst);

    // Many intervals' worth of time; a still-running loop would commit again.
    tokio::time::sleep(interval * 10).await;
    assert_eq!(
        client.commit_calls.load(Ordering::SeqCst),
        after_finalize,
        "the coordinator must stop after finalize rather than keep checkpointing"
    );
}

#[tokio::test]
async fn coordinator_failed_attempt_returns_to_idle_and_retries_next_interval() {
    // Failure is retried by the ordinary periodic timer rather than a distinct backoff, so a
    // failing attempt must neither wedge the loop nor spin.
    let client = FailingUploadTargetsClient::new();
    let task_id = fresh_task_id();
    let tempdir = TempDir::new().unwrap();
    let file_path = tempdir.path().join("note.txt");
    std::fs::write(&file_path, b"hi").unwrap();
    let _decl = DeclarationsFixture::new(&task_id, &file_path);

    let always_safe: BoundaryCheck = Arc::new(|| Box::pin(async { Boundary::Safe }));
    let handle = CheckpointCoordinatorHandle::new_for_test(
        client.clone(),
        task_id,
        tempdir.path().to_path_buf(),
        always_safe,
        Duration::from_millis(20),
        Duration::from_millis(10),
        Duration::from_secs(5),
        test_background(),
    );

    wait_for(
        || client.call_count.load(Ordering::SeqCst) >= 2,
        Duration::from_secs(10),
    )
    .await;
    drop(handle);
}

#[tokio::test]
async fn coordinator_finalize_from_idle_skips_attempt_below_floor() {
    let client = FailingUploadTargetsClient::new();
    let task_id = fresh_task_id();
    let tempdir = TempDir::new().unwrap();
    // No declarations fixture: the client would panic on any upload-targets/commit call, so
    // this also proves no attempt is made.
    let always_safe: BoundaryCheck = Arc::new(|| Box::pin(async { Boundary::Safe }));

    let handle = CheckpointCoordinatorHandle::new_for_test(
        client.clone(),
        task_id,
        tempdir.path().to_path_buf(),
        always_safe,
        Duration::from_secs(600), // never fires during this test
        Duration::from_secs(5),
        Duration::from_secs(5),
        test_background(),
    );

    // floor = 10s; a 100ms budget is well below it.
    handle.finalize(Duration::from_millis(100)).await;

    assert_eq!(
        client.call_count.load(Ordering::SeqCst),
        0,
        "a below-floor finalize budget from Idle must skip the attempt entirely"
    );
}
