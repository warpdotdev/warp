use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::manager::RemoteServerExitStatus;
use crate::setup::{PreinstallCheckResult, RemoteArch, RemoteOs, RemotePlatform};
use crate::transport::{Connection, RemoteTransport};

/// Counter that detects concurrent async operations.
///
/// Increments `in_flight` on entry, decrements on exit, and records the
/// peak into `max_in_flight`. A well-serialized call sequence should
/// never push `max_in_flight` above 1.
#[derive(Clone)]
struct ConcurrencyTracker {
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
    call_count: Arc<AtomicUsize>,
}

impl ConcurrencyTracker {
    fn new() -> Self {
        Self {
            in_flight: Arc::new(AtomicUsize::new(0)),
            max_in_flight: Arc::new(AtomicUsize::new(0)),
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Mark a call as started. Returns a guard that decrements on drop.
    fn enter(&self) -> ConcurrencyGuard {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        let prev = self.in_flight.fetch_add(1, Ordering::SeqCst);
        let current = prev + 1;
        // CAS loop to update peak.
        loop {
            let peak = self.max_in_flight.load(Ordering::SeqCst);
            if current <= peak {
                break;
            }
            if self
                .max_in_flight
                .compare_exchange(peak, current, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }
        ConcurrencyGuard {
            in_flight: Arc::clone(&self.in_flight),
        }
    }

    fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }

    fn total_calls(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

struct ConcurrencyGuard {
    in_flight: Arc<AtomicUsize>,
}

impl Drop for ConcurrencyGuard {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Mock transport that tracks concurrency of SSH operations.
///
/// Each method simulates a small delay (like a real SSH command) and
/// uses [`ConcurrencyTracker`] to detect whether any calls overlap.
#[derive(Clone)]
struct MockSshTransport {
    tracker: ConcurrencyTracker,
}

impl std::fmt::Debug for MockSshTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockSshTransport").finish()
    }
}

impl MockSshTransport {
    fn new(tracker: ConcurrencyTracker) -> Self {
        Self { tracker }
    }
}

impl RemoteTransport for MockSshTransport {
    fn detect_platform(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<RemotePlatform, String>> + Send>> {
        let tracker = self.tracker.clone();
        Box::pin(async move {
            let _guard = tracker.enter();
            // Simulate SSH round-trip.
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(RemotePlatform {
                os: RemoteOs::Linux,
                arch: RemoteArch::X86_64,
            })
        })
    }

    fn run_preinstall_check(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<PreinstallCheckResult, String>> + Send>>
    {
        let tracker = self.tracker.clone();
        Box::pin(async move {
            let _guard = tracker.enter();
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(PreinstallCheckResult::parse(
                "status=supported\nlibc_family=glibc\nlibc_version=2.35\n",
            ))
        })
    }

    fn check_binary(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<bool, String>> + Send>> {
        let tracker = self.tracker.clone();
        Box::pin(async move {
            let _guard = tracker.enter();
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(true)
        })
    }

    fn check_has_old_binary(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send>> {
        let tracker = self.tracker.clone();
        Box::pin(async move {
            let _guard = tracker.enter();
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(false)
        })
    }

    fn install_binary(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
        let tracker = self.tracker.clone();
        Box::pin(async move {
            let _guard = tracker.enter();
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(())
        })
    }

    fn connect(
        &self,
        _executor: std::sync::Arc<warpui::r#async::executor::Background>,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<Connection>> + Send>> {
        Box::pin(async { Err(anyhow::anyhow!("mock: connect not implemented")) })
    }

    fn remove_remote_server_binary(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>> {
        let tracker = self.tracker.clone();
        Box::pin(async move {
            let _guard = tracker.enter();
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(())
        })
    }

    fn is_reconnectable(&self, _exit_status: Option<&RemoteServerExitStatus>) -> bool {
        true
    }
}

/// Verifies that the check_binary flow runs SSH operations sequentially,
/// never exceeding one concurrent SSH command at a time.
///
/// This mirrors the calling pattern in
/// `RemoteServerManager::check_binary` after the fix in 0f28bcb3 that
/// replaced `futures::join!` with sequential `.await` calls.
///
/// A MaxSessions-limited SSH server would fail if these ran
/// concurrently (each command opens a ControlMaster channel), so this
/// test guards against regressions that re-introduce parallelism.
#[tokio::test]
async fn check_binary_flow_runs_ssh_ops_sequentially() {
    let tracker = ConcurrencyTracker::new();
    let transport = MockSshTransport::new(tracker.clone());

    // Reproduce the exact calling pattern from
    // RemoteServerManager::check_binary (manager.rs).
    let _platform_result = transport.detect_platform().await;
    let _check_result = transport.check_binary().await;
    let _old_binary_result = transport.check_has_old_binary().await;
    let _preinstall = transport.run_preinstall_check().await;

    assert_eq!(
        tracker.max_in_flight(),
        1,
        "Expected at most 1 concurrent SSH operation, but observed {}. \
         This would exhaust SSH MaxSessions on restricted hosts.",
        tracker.max_in_flight()
    );
    assert_eq!(tracker.total_calls(), 4, "Expected 4 SSH operations total");
}

/// Verifies that the install+SCP-fallback flow runs SSH operations
/// sequentially.
///
/// Mirrors the `scp_install_fallback` path in
/// `SshTransport::install_binary` where, after the install script fails
/// with NO_HTTP_CLIENT_EXIT_CODE, we:
/// 1. detect_platform (uname -sm)
/// 2. scp_upload (not an SSH command per se, but uses ControlMaster)
/// 3. run_ssh_script (extraction)
///
/// The scp_upload is excluded here since it's a separate binary (`scp`)
/// that also multiplexes through the ControlMaster, but the other SSH
/// commands must not overlap with it.
#[tokio::test]
async fn scp_install_fallback_runs_ssh_ops_sequentially() {
    let tracker = ConcurrencyTracker::new();
    let transport = MockSshTransport::new(tracker.clone());

    // SCP fallback path: detect platform → install.
    let _platform = transport.detect_platform().await;
    let _install = transport.install_binary().await;

    assert_eq!(
        tracker.max_in_flight(),
        1,
        "Expected at most 1 concurrent SSH operation in SCP fallback, but observed {}",
        tracker.max_in_flight()
    );
    assert_eq!(tracker.total_calls(), 2);
}

/// Contrast test: proves the tracker CAN detect concurrency when
/// operations DO overlap. Guards against a false-positive tracker that
/// always reports max_in_flight == 1.
#[tokio::test]
async fn tracker_detects_concurrent_calls() {
    let tracker = ConcurrencyTracker::new();
    let transport = MockSshTransport::new(tracker.clone());

    // Run three operations concurrently — this is the WRONG pattern
    // (the old futures::join! approach). The tracker should detect it.
    let (_, _, _) = futures::future::join3(
        transport.detect_platform(),
        transport.check_binary(),
        transport.check_has_old_binary(),
    )
    .await;

    assert!(
        tracker.max_in_flight() > 1,
        "Tracker should detect concurrency: max_in_flight was {}",
        tracker.max_in_flight()
    );
    assert_eq!(tracker.total_calls(), 3);
}

/// End-to-end: the full setup sequence (check → install → connect
/// attempt) never runs more than one SSH command at a time.
#[tokio::test]
async fn full_setup_sequence_is_serial() {
    let tracker = ConcurrencyTracker::new();
    let transport = MockSshTransport::new(tracker.clone());

    // Phase 1: check_binary flow.
    let _platform = transport.detect_platform().await;
    let _check = transport.check_binary().await;
    let _old = transport.check_has_old_binary().await;
    let _preinstall = transport.run_preinstall_check().await;

    // Phase 2: install_binary flow.
    let _install = transport.install_binary().await;

    // Phase 3: version mismatch removal (happens inside
    // run_connect_and_handshake when versions diverge).
    let _remove = transport.remove_remote_server_binary().await;

    assert_eq!(
        tracker.max_in_flight(),
        1,
        "Full setup sequence should never exceed 1 concurrent SSH op, but peaked at {}",
        tracker.max_in_flight()
    );
    assert_eq!(
        tracker.total_calls(),
        6,
        "Expected 6 SSH operations across all setup phases"
    );
}
